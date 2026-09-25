//! Session shell. The bottom prompt always talks to the view that is open:
//! the desk, the selected case, or the module on the canvas.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;

use anyhow::Result;
use argos_osint_core::agent::{self, HistMsg, TurnEvent, TurnInput};
use argos_osint_core::brain::{self, Memory};
use argos_osint_core::gmail::{self, GmailConfig};
use argos_osint_core::hardware::{self, HardwareProfile};
use argos_osint_core::paths::{self, db_label};
use argos_osint_core::prompt::{self, Intent};
use argos_osint_core::provider::{self, SettingsFile};
use argos_osint_core::report::{self, ReportMeta};
use argos_osint_core::search::{SearchHit, SourcePlan};
use argos_osint_core::secrets::{self, AuthFile, GmailSecret, ProviderSecret};
use argos_osint_core::session::{self, Case};
use argos_osint_core::store::{ChatLine, Store};
use argos_osint_core::tna::{self, desk_key, report_key, TnaCluster, TnaNode, TnaNodeKind, TnaSnapshot};
use chrono::Local;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers, MouseEventKind};
use ratatui::layout::Rect;
use sysinfo::System;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ModuleId {
    Cases,
    System,
    Hardware,
    Providers,
    Osint,
    Brain,
    Gmail,
    Reports,
    Log,
    Settings,
}

impl ModuleId {
    pub fn title(self) -> &'static str {
        match self {
            Self::Cases => "Case Desk",
            Self::System => "System",
            Self::Hardware => "Hardware",
            Self::Providers => "Providers",
            Self::Osint => "OSINT Providers",
            Self::Brain => "Brain",
            Self::Gmail => "Gmail",
            Self::Reports => "Reports",
            Self::Log => "Log",
            Self::Settings => "Settings",
        }
    }

    pub fn blurb(self) -> &'static str {
        match self {
            Self::Cases => "Desk for new queries, with reports beside it",
            Self::System => "Log, hardware, and settings",
            Self::Hardware => "Cores, RAM, VRAM, architecture",
            Self::Providers => "Mail, OSINT sources, and LLM login",
            Self::Osint => "Public search sources for case research",
            Self::Brain => "fact, identity, preference, contact, project, goal, task",
            Self::Gmail => "Gmail IMAP app password and MCP",
            Self::Reports => "Markdown reports on disk",
            Self::Log => "Timestamped system, API, task, and search log",
            Self::Settings => "SearXNG URL and report folder",
        }
    }

    /// Launcher order: case desk, providers, system.
    /// Hardware and Settings are tabs on System. Log is the System log.
    pub fn all() -> [ModuleId; 3] {
        [Self::Cases, Self::Providers, Self::System]
    }

    pub fn from_name(name: &str) -> Option<Self> {
        let n = name.trim().to_lowercase();
        Self::all()
            .into_iter()
            .chain([
                Self::Brain,
                Self::Gmail,
                Self::Osint,
                Self::Reports,
                Self::Log,
                Self::Hardware,
                Self::Settings,
            ])
            .find(|m| {
                let title = m.title().to_lowercase();
                title == n
                    || title.starts_with(&n)
                    || format!("{:?}", m).to_lowercase() == n
                    || (n == "chat" && *m == Self::Providers)
                    || (n == "osint" && *m == Self::Osint)
            })
    }
}

/// Side pages on the case desk. Reports stay beside the desk and are not a page.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CasePage {
    Closed,
    Brain,
    Network,
}

impl CasePage {
    pub fn all() -> [Self; 3] {
        [Self::Closed, Self::Brain, Self::Network]
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Closed => "Desk",
            Self::Brain => "Brain",
            Self::Network => "Network",
        }
    }
}

/// Network canvas presentation mode (FR-4).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TnaView {
    Graph,
    Outline,
    Table,
}

impl TnaView {
    pub fn next(self) -> Self {
        match self {
            Self::Graph => Self::Outline,
            Self::Outline => Self::Table,
            Self::Table => Self::Graph,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Graph => "graph",
            Self::Outline => "outline",
            Self::Table => "table",
        }
    }
}

/// Selectable item on the Graph canvas after egocentric collapse.
#[derive(Clone, Debug, PartialEq)]
pub enum TnaDisplayItem {
    Real { idx: usize },
    Super {
        hub_id: String,
        count: usize,
        x: f64,
        y: f64,
    },
}

/// Glyph for a node kind / cluster (FR-4 readable canvas).
pub fn tna_glyph_for_kind(kind: TnaNodeKind) -> &'static str {
    match kind.cluster() {
        TnaCluster::Infrastructure => "◆",
        TnaCluster::Campaign => "▲",
        TnaCluster::Identity => "●",
        TnaCluster::FiledReports => "□",
    }
}

pub fn tna_glyph_for_cluster(cluster: TnaCluster) -> &'static str {
    match cluster {
        TnaCluster::Infrastructure => "◆",
        TnaCluster::Campaign => "▲",
        TnaCluster::Identity => "●",
        TnaCluster::FiledReports => "□",
    }
}

pub fn tna_hub_threshold(nodes: &[TnaNode]) -> u32 {
    let max_deg = nodes.iter().map(|n| n.degree).max().unwrap_or(0);
    if max_deg < 15 {
        8
    } else {
        15
    }
}

/// Tabs on the System app. Log replaces the old case-desk search log.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SystemPage {
    Log,
    Hardware,
    Settings,
}

impl SystemPage {
    pub fn all() -> [Self; 3] {
        [Self::Log, Self::Hardware, Self::Settings]
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Log => "Log",
            Self::Hardware => "Hardware",
            Self::Settings => "Settings",
        }
    }
}

/// One timestamped line in the System log.
#[derive(Clone, Debug)]
pub struct LogEntry {
    pub at: String,
    pub kind: String,
    pub text: String,
}

/// Stage toggles for one case worker. Copied from settings, then changed
/// only for this run.
#[derive(Clone, Debug)]
pub struct ScopeDraft {
    pub query: String,
    pub echo_on_desk: bool,
    pub restore_prompt: bool,
    pub facts: bool,
    pub web: bool,
    pub news: bool,
    pub domain: bool,
    pub social: bool,
    pub identity: bool,
    pub selected: usize,
}

/// Which saved role the model card writes to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ModelTarget {
    /// The connection model on the provider.
    Connection,
    /// The model that answers the user.
    Writer,
    /// The model that calls research tools.
    Tool,
}

/// Research that has started and has not filed markdown yet.
/// `failed` is set when the worker stops without a report.
#[derive(Clone, Debug)]
pub struct PendingReport {
    pub case_id: String,
    pub title: String,
    pub failed: Option<String>,
}

/// One row in the report list beside the case desk.
#[derive(Clone, Debug)]
pub enum ReportRow {
    Pending {
        case_id: String,
        title: String,
        failed: Option<String>,
    },
    Completed(ReportMeta),
}

impl ReportRow {
    pub fn title(&self) -> &str {
        match self {
            Self::Pending { title, .. } => title,
            Self::Completed(report) => &report.title,
        }
    }

    pub fn case_id(&self) -> Option<&str> {
        match self {
            Self::Pending { case_id, .. } => Some(case_id),
            Self::Completed(report) => report.case_id.as_deref(),
        }
    }

    pub fn status(&self) -> &'static str {
        match self {
            Self::Pending {
                failed: Some(_), ..
            } => "failed",
            Self::Pending { .. } => "pending",
            Self::Completed(_) => "completed",
        }
    }

    pub fn when(&self) -> Option<String> {
        match self {
            Self::Completed(report) => Some(report::short_when(&report.created_at)),
            _ => None,
        }
    }
}

/// Side pages on Providers: mail, OSINT sources, and the LLM login.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ProviderPage {
    Mail,
    Osint,
    Llm,
}

impl ProviderPage {
    pub fn all() -> [Self; 3] {
        [Self::Mail, Self::Osint, Self::Llm]
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Mail => "Mail",
            Self::Osint => "OSINT",
            Self::Llm => "LLM",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Focus {
    Launcher,
    Canvas,
    Reports,
    Prompt,
    Graph,
    TnaSide,
}

#[derive(Clone, Debug)]
pub struct Field {
    pub key: String,
    pub label: String,
    pub value: String,
    pub secret: bool,
}

#[derive(Clone, Debug)]
pub enum AppMsg {
    Turn(TurnEvent),
    Hardware(HardwareProfile),
    Note(String),
    Search {
        query: String,
        result: Result<Vec<SearchHit>, String>,
    },
    Models(Result<Vec<String>, String>),
    ModelList(Result<Vec<provider::ListedModel>, String>),
    Voice(Result<String, String>),
    GmailTest(Result<String, String>),
    Research {
        case_id: String,
        result: Result<(String, Option<ReportMeta>), String>,
    },
    Insight {
        report_id: String,
        fact: String,
    },
}

pub struct App {
    pub tx: UnboundedSender<AppMsg>,
    pub store: Store,
    pub settings: SettingsFile,
    pub auth: AuthFile,
    pub focus: Focus,
    pub launcher_sel: usize,
    pub module: Option<ModuleId>,
    pub case_page: CasePage,
    pub system_page: SystemPage,
    pub provider_page: ProviderPage,
    pub source_sel: usize,
    pub modal: bool,
    pub modal_query: String,
    pub modal_sel: usize,
    pub help: bool,
    /// Query waiting on the case-worker confirmation popup.
    pub confirm_query: Option<String>,
    pub confirm_sel: usize,
    /// Source toggles for the case worker that is about to start.
    pub scope: Option<ScopeDraft>,
    /// Completed report waiting on the open-chat confirmation popup.
    pub confirm_report: Option<String>,
    pub confirm_report_sel: usize,
    /// Fact text held when a desk question already has memories but the user asked for a new case.
    pub desk_memory_answer: Option<String>,
    /// Second confirmation before a report file and its facts are removed.
    pub confirm_delete_report: Option<String>,
    pub confirm_delete_sel: usize,
    pub model_picker: bool,
    pub model_target: ModelTarget,
    pub model_query: String,
    pub model_sel: usize,
    pub remote_models: Vec<String>,
    pub catalog: Vec<provider::ListedModel>,
    /// Concrete models behind `openrouter/free`.
    pub free_picker: bool,
    pub free_query: String,
    pub free_sel: usize,
    pub free_loading: bool,
    pub prompt: String,
    pub cursor: usize,
    pub history: Vec<String>,
    pub hist_pos: Option<usize>,
    pub transcripts: HashMap<String, Vec<ChatLine>>,
    pub cases: Vec<Case>,
    pub case_sel: usize,
    /// None means the prompt is talking to the case desk. Some is a selected case.
    pub chat_case: Option<String>,
    /// Completed report whose chat replaces the case-desk canvas.
    pub chat_report: Option<String>,
    pub memories: Vec<Memory>,
    pub brain_sel: usize,
    /// `all` or one memory category. Filters the Brain list.
    pub brain_filter: String,
    /// Set while the editor is updating an existing memory.
    pub brain_edit_id: Option<String>,
    /// Popup card for creating or editing one memory.
    pub brain_card: bool,
    /// 0 edit, 1 save, 2 cancel, 3 delete.
    pub brain_action: usize,
    pub reports: Vec<ReportMeta>,
    /// Research that has started and has not filed a markdown report yet.
    pub pending_reports: Vec<PendingReport>,
    pub report_sel: usize,
    pub log: Vec<LogEntry>,
    pub hardware: HardwareProfile,
    pub cpu_now: f32,
    pub cpu_hist: VecDeque<u64>,
    pub sys: System,
    pub fields: Vec<Field>,
    pub field_sel: usize,
    pub editing: bool,
    pub provider_slot: &'static str,
    pub running: bool,
    pub cancel: Arc<AtomicBool>,
    pub spinner: usize,
    pub tick_n: u64,
    pub status: String,
    pub db_label: String,
    pub scroll_back: usize,
    pub quit: bool,
    pub launcher_area: Rect,
    pub report_area: Rect,
    pub canvas_area: Rect,
    /// Content rows inside the reports pane. `Some(index)` is a clickable report.
    pub report_line_index: Vec<Option<usize>>,
    pub case_tab_area: Rect,
    pub provider_tab_area: Rect,
    /// Clickable label for each Case Desk tab, in tab order.
    pub case_tab_hits: Vec<Rect>,
    /// Clickable label for each Providers tab, in tab order.
    pub provider_tab_hits: Vec<Rect>,
    pub system_tab_area: Rect,
    /// Clickable label for each System tab, in tab order.
    pub system_tab_hits: Vec<Rect>,
    pub tna_desk: Option<TnaSnapshot>,
    pub tna_report: Option<TnaSnapshot>,
    pub tna_find: Option<String>,
    pub tna_sel: usize,
    pub tna_side_scroll: usize,
    pub tna_rebuilding: bool,
    pub tna_view: TnaView,
    /// Hub node ids whose far neighbors are expanded (not collapsed to ▣×N).
    pub tna_expanded: HashSet<String>,
    /// Egocentric pin for Graph collapse (node id).
    pub tna_focus_id: Option<String>,
}

impl App {
    pub fn boot() -> Result<Self> {
        paths::ensure_home()?;
        let store = Store::open(&paths::db_path())?;
        store.ensure_session("desk", "Desk", "desk")?;
        store.clear_messages("desk")?;
        for module in ModuleId::all() {
            if module != ModuleId::Cases {
                store.ensure_session(&module_session(module), module.title(), "module")?;
            }
        }
        let (settings, config_error) = match SettingsFile::load() {
            Ok(settings) => (settings, None),
            Err(err) => (SettingsFile::default(), Some(err.to_string())),
        };
        let auth = AuthFile::load().unwrap_or_default();
        let mut app = Self::from_parts(store, settings, auth)?;
        if let Some(err) = config_error {
            app.log_event("system", &format!("config load failed: {err}"));
        }
        app.reload_lists()?;
        Ok(app)
    }

    pub fn from_parts(store: Store, settings: SettingsFile, auth: AuthFile) -> Result<Self> {
        let (tx, _rx) = unbounded_channel();
        let mut app = Self {
            tx,
            store,
            settings,
            auth,
            focus: Focus::Prompt,
            launcher_sel: 0,
            module: Some(ModuleId::Cases),
            case_page: CasePage::Closed,
            system_page: SystemPage::Log,
            provider_page: ProviderPage::Llm,
            source_sel: 0,
            modal: false,
            modal_query: String::new(),
            modal_sel: 0,
            help: false,
            confirm_query: None,
            confirm_sel: 0,
            scope: None,
            confirm_report: None,
            confirm_report_sel: 0,
            desk_memory_answer: None,
            confirm_delete_report: None,
            confirm_delete_sel: 1,
            model_picker: false,
            model_target: ModelTarget::Connection,
            model_query: String::new(),
            model_sel: 0,
            remote_models: Vec::new(),
            catalog: Vec::new(),
            free_picker: false,
            free_query: String::new(),
            free_sel: 0,
            free_loading: false,
            prompt: String::new(),
            cursor: 0,
            history: Vec::new(),
            hist_pos: None,
            transcripts: HashMap::new(),
            cases: Vec::new(),
            case_sel: 0,
            chat_case: None,
            chat_report: None,
            memories: Vec::new(),
            brain_sel: 0,
            brain_filter: "all".into(),
            brain_edit_id: None,
            brain_card: false,
            brain_action: 0,
            reports: Vec::new(),
            pending_reports: Vec::new(),
            report_sel: 0,
            log: Vec::new(),
            hardware: HardwareProfile::unknown(),
            cpu_now: 0.0,
            cpu_hist: VecDeque::new(),
            sys: System::new(),
            fields: Vec::new(),
            field_sel: 0,
            editing: false,
            provider_slot: "text",
            running: false,
            cancel: Arc::new(AtomicBool::new(false)),
            spinner: 0,
            tick_n: 0,
            status: "ready".into(),
            db_label: db_label(),
            scroll_back: 0,
            quit: false,
            launcher_area: Rect::default(),
            report_area: Rect::default(),
            canvas_area: Rect::default(),
            report_line_index: Vec::new(),
            case_tab_area: Rect::default(),
            provider_tab_area: Rect::default(),
            case_tab_hits: Vec::new(),
            provider_tab_hits: Vec::new(),
            system_tab_area: Rect::default(),
            system_tab_hits: Vec::new(),
            tna_desk: None,
            tna_report: None,
            tna_find: None,
            tna_sel: 0,
            tna_side_scroll: 0,
            tna_rebuilding: false,
            tna_view: TnaView::Graph,
            tna_expanded: HashSet::new(),
            tna_focus_id: None,
        };
        app.reload_lists()?;
        app.load_transcript(&app.session_id());
        Ok(app)
    }

    pub fn take_inbox(&mut self) -> UnboundedReceiver<AppMsg> {
        let (tx, rx) = unbounded_channel();
        self.tx = tx;
        rx
    }

    pub fn reload_lists(&mut self) -> Result<()> {
        self.cases = self.store.list_cases()?;
        self.memories = self.store.list_memories()?;
        self.reports = self.store.list_reports()?;
        if self.case_sel >= self.cases.len() && !self.cases.is_empty() {
            self.case_sel = 0;
        }
        if let Some(id) = &self.chat_case {
            if !self.cases.iter().any(|case| &case.id == id) {
                self.chat_case = None;
            }
        }
        if let Some(id) = &self.chat_report {
            if !self.reports.iter().any(|report| &report.id == id) {
                self.chat_report = None;
            }
        }
        let rows = self.report_rows().len();
        if rows == 0 {
            self.report_sel = 0;
        } else if self.report_sel >= rows {
            self.report_sel = rows - 1;
        }
        Ok(())
    }

    /// Chat lands on the open report, the selected case, or the case desk.
    pub fn session_id(&self) -> String {
        if let Some(id) = &self.chat_report {
            return format!("report:{id}");
        }
        self.chat_case.clone().unwrap_or_else(|| "desk".into())
    }

    pub fn view_name(&self) -> String {
        if let Some(report) = self.open_report() {
            return format!("Report · {}", report.title);
        }
        match self
            .chat_case
            .as_ref()
            .and_then(|id| self.cases.iter().find(|case| &case.id == id))
        {
            Some(case) => format!("Case · {}", case.title),
            None => "Case Desk".into(),
        }
    }

    pub fn open_report(&self) -> Option<&ReportMeta> {
        let id = self.chat_report.as_deref()?;
        self.reports.iter().find(|report| report.id == id)
    }

    pub fn widget(&self) -> Option<ModuleId> {
        match self.module {
            Some(ModuleId::Cases) => match self.case_page {
                CasePage::Closed => None,
                CasePage::Brain => Some(ModuleId::Brain),
                CasePage::Network => None,
            },
            Some(ModuleId::System) => match self.system_page {
                SystemPage::Log => Some(ModuleId::Log),
                SystemPage::Hardware => Some(ModuleId::Hardware),
                SystemPage::Settings => Some(ModuleId::Settings),
            },
            Some(ModuleId::Providers) => Some(ModuleId::Providers),
            None
            | Some(ModuleId::Brain)
            | Some(ModuleId::Gmail)
            | Some(ModuleId::Osint)
            | Some(ModuleId::Reports) => None,
            Some(module) => Some(module),
        }
    }

    /// The side page whose fields and keys are live.
    pub fn form_module(&self) -> Option<ModuleId> {
        match self.module {
            Some(ModuleId::Cases) => match self.case_page {
                CasePage::Brain => Some(ModuleId::Brain),
                CasePage::Closed | CasePage::Network => None,
            },
            Some(ModuleId::System) => match self.system_page {
                SystemPage::Settings => Some(ModuleId::Settings),
                SystemPage::Hardware => Some(ModuleId::Hardware),
                SystemPage::Log => None,
            },
            Some(ModuleId::Providers) => match self.provider_page {
                ProviderPage::Mail => Some(ModuleId::Gmail),
                ProviderPage::Osint => Some(ModuleId::Osint),
                ProviderPage::Llm => Some(ModuleId::Providers),
            },
            Some(ModuleId::Settings) => Some(ModuleId::Settings),
            Some(ModuleId::Hardware) => Some(ModuleId::Hardware),
            _ => None,
        }
    }

    pub fn mode_label(&self) -> String {
        let run = if self.running {
            let frames = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
            format!(" {} {}", frames[self.spinner % frames.len()], self.status)
        } else {
            String::new()
        };
        format!(
            "{} · {} · {}{run}",
            self.view_name(),
            self.active_model(),
            self.settings.modality,
        )
    }

    pub fn text_secret(&self) -> argos_osint_core::secrets::ProviderSecret {
        provider::active_text_secret(&self.auth, &self.settings.model)
    }

    pub fn active_model(&self) -> String {
        self.role_secret(&self.settings.writer_model).model
    }

    /// Connection provider with a role model, or the connection model when the role is empty.
    pub fn role_secret(&self, model: &str) -> argos_osint_core::secrets::ProviderSecret {
        let mut secret = self.text_secret();
        if !model.trim().is_empty() {
            secret.model = model.trim().to_string();
        }
        secret
    }

    fn role_label(&self, model: &str) -> String {
        if model.trim().is_empty() {
            format!("connection ({})", self.text_secret().model)
        } else {
            model.trim().to_string()
        }
    }

    pub fn model_choices(&self) -> Vec<(String, String)> {
        let secret = self.text_secret();
        let mut choices = Vec::new();
        if provider::effective_kind(&secret) == "grok" {
            for model in provider::grok_models() {
                choices.push((model.id.to_string(), model.name.to_string()));
            }
        }
        for id in &self.remote_models {
            if !choices.iter().any(|(existing, _)| existing == id) {
                choices.push((id.clone(), id.clone()));
            }
        }
        if choices.is_empty() {
            choices.push((secret.model.clone(), secret.model.clone()));
        }
        choices
    }

    pub fn filtered_model_choices(&self) -> Vec<(String, String)> {
        let q = self.model_query.trim().to_lowercase();
        self.model_choices()
            .into_iter()
            .filter(|(id, label)| {
                q.is_empty() || id.to_lowercase().contains(&q) || label.to_lowercase().contains(&q)
            })
            .collect()
    }

    fn open_model_picker(&mut self) {
        self.model_target = ModelTarget::Connection;
        self.open_model_card();
    }

    fn open_model_card(&mut self) {
        self.model_picker = true;
        self.free_picker = false;
        self.model_query.clear();
        self.model_sel = 0;
        self.refresh_model_list();
    }

    fn refresh_model_list(&self) {
        let secret = self.text_secret();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = provider::list_catalog(&secret)
                .await
                .map_err(|err| err.to_string());
            let _ = tx.send(AppMsg::ModelList(result));
        });
    }

    pub fn filtered_free_models(&self) -> Vec<(String, String)> {
        let q = self.free_query.trim().to_lowercase();
        provider::concrete_free_models(&self.catalog)
            .into_iter()
            .filter(|model| {
                q.is_empty()
                    || model.id.to_lowercase().contains(&q)
                    || model.name.to_lowercase().contains(&q)
            })
            .map(|model| (model.id, model.name))
            .collect()
    }

    fn open_free_picker(&mut self) {
        self.free_picker = true;
        self.free_query.clear();
        self.free_sel = 0;
        if self.filtered_free_models().is_empty() {
            self.free_loading = true;
            self.refresh_model_list();
        }
    }

    pub(crate) fn select_model(&mut self, id: &str) {
        if provider::is_free_router(id) {
            self.open_free_picker();
            return;
        }
        match self.model_target {
            ModelTarget::Writer => {
                self.settings.writer_model = id.to_string();
                let _ = self.settings.save();
                self.status = format!("writer {id}");
                self.log_event("system", &format!("writer model {id}"));
            }
            ModelTarget::Tool => {
                self.settings.tool_model = id.to_string();
                let _ = self.settings.save();
                self.status = format!("tool caller {id}");
                self.log_event("system", &format!("tool model {id}"));
            }
            ModelTarget::Connection => {
                self.settings.model = id.to_string();
                let _ = self.settings.save();
                match self.auth.text.as_mut() {
                    Some(slot) => slot.model = id.to_string(),
                    None => {
                        let mut secret = self.text_secret();
                        secret.model = id.to_string();
                        self.auth.text = Some(secret);
                    }
                }
                let _ = self.auth.save();
                self.status = format!("model {id}");
                self.log_event("system", &format!("model set to {id}"));
            }
        }
        self.model_picker = false;
        self.free_picker = false;
        if self.form_module() == Some(ModuleId::Providers) {
            self.load_fields(ModuleId::Providers);
        }
    }

    pub fn picker_current(&self) -> String {
        match self.model_target {
            ModelTarget::Writer => self.settings.writer_model.trim().to_string(),
            ModelTarget::Tool => self.settings.tool_model.trim().to_string(),
            ModelTarget::Connection => self.text_secret().model,
        }
    }

    pub fn picker_title(&self) -> &'static str {
        match self.model_target {
            ModelTarget::Writer => "Writer model",
            ModelTarget::Tool => "Tool model",
            ModelTarget::Connection => "Models",
        }
    }

    fn on_model_key(&mut self, key: KeyEvent) -> bool {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if key.code == KeyCode::Esc || (ctrl && key.code == KeyCode::Char('m')) {
            self.model_picker = false;
            return false;
        }
        let count = self.filtered_model_choices().len();
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                if count > 0 {
                    self.model_sel = (self.model_sel + count - 1) % count;
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if count > 0 {
                    self.model_sel = (self.model_sel + 1) % count;
                }
            }
            KeyCode::Enter => {
                if let Some((id, _)) = self.filtered_model_choices().get(self.model_sel) {
                    let id = id.clone();
                    self.select_model(&id);
                }
            }
            KeyCode::Backspace => {
                self.model_query.pop();
                self.model_sel = 0;
            }
            KeyCode::Char(ch) if !ctrl => {
                self.model_query.push(ch);
                self.model_sel = 0;
            }
            _ => {}
        }
        false
    }

    fn on_free_key(&mut self, key: KeyEvent) -> bool {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if key.code == KeyCode::Esc {
            self.free_picker = false;
            self.free_loading = false;
            return false;
        }
        let count = self.filtered_free_models().len();
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                if count > 0 {
                    self.free_sel = (self.free_sel + count - 1) % count;
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if count > 0 {
                    self.free_sel = (self.free_sel + 1) % count;
                }
            }
            KeyCode::Enter => {
                if let Some((id, _)) = self.filtered_free_models().get(self.free_sel) {
                    let id = id.clone();
                    self.select_model(&id);
                }
            }
            KeyCode::Backspace => {
                self.free_query.pop();
                self.free_sel = 0;
            }
            KeyCode::Char(ch) if !ctrl => {
                self.free_query.push(ch);
                self.free_sel = 0;
            }
            _ => {}
        }
        false
    }

    pub fn transcript(&self) -> &[ChatLine] {
        self.transcripts
            .get(&self.session_id())
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    fn load_transcript(&mut self, id: &str) {
        if self.transcripts.contains_key(id) {
            return;
        }
        let lines = self.store.load_messages(id).unwrap_or_default();
        self.transcripts.insert(id.to_string(), lines);
    }

    fn push_line(&mut self, role: &str, body: &str) {
        let id = self.session_id();
        let _ = self.store.append_message(&id, role, body);
        let line = ChatLine {
            role: role.into(),
            body: body.into(),
            created_at: String::new(),
        };
        self.transcripts.entry(id).or_default().push(line);
        self.scroll_back = 0;
    }

    fn capture_report_insight(&mut self, answer: &str) {
        let Some(report_id) = self.chat_report.clone() else {
            return;
        };
        let answer = answer.trim().to_string();
        if answer.is_empty() {
            return;
        }
        let question = self
            .transcript()
            .iter()
            .rev()
            .find(|line| line.role == "user")
            .map(|line| line.body.clone())
            .unwrap_or_default();
        let title = self
            .reports
            .iter()
            .find(|report| report.id == report_id)
            .map(|report| report.title.clone())
            .unwrap_or_else(|| "report".into());
        let secret = self.text_secret();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let fact = match distill_insight(&secret, &title, &question, &answer).await {
                Ok(text) => text,
                Err(_) => brain::clip_fact(&answer),
            };
            let fact = brain::clip_fact(&fact);
            if fact.is_empty() {
                return;
            }
            let _ = tx.send(AppMsg::Insight { report_id, fact });
        });
    }

    fn store_report_insight(&mut self, report_id: String, fact: String) {
        let title = self
            .reports
            .iter()
            .find(|report| report.id == report_id)
            .map(|report| report.title.clone())
            .unwrap_or_else(|| report_id.clone());
        let text = if fact.contains("(report:") {
            fact
        } else {
            format!("{fact} (report: {title})")
        };
        if self.memories.iter().any(|memory| {
            memory.report_id.as_deref() == Some(report_id.as_str()) && memory.text == text
        }) {
            return;
        }
        if let Ok(memory) = self.store.add_report_fact(&text, &report_id) {
            self.memories.insert(0, memory);
            self.log_event("system", "fact filed from report chat");
        }
    }

    fn replace_last_assistant(&mut self, body: &str) {
        let id = self.session_id();
        let _ = self.store.update_last_message(&id, "assistant", body);
        if let Some(lines) = self.transcripts.get_mut(&id) {
            if let Some(last) = lines.last_mut() {
                if last.role == "assistant" {
                    last.body = body.to_string();
                    return;
                }
            }
        }
        self.push_line("assistant", body);
    }

    /// Write the reply currently on screen so a later visit reloads it.
    fn persist_visible_chat(&mut self) {
        let id = self.session_id();
        let Some(body) = self
            .transcripts
            .get(&id)
            .and_then(|lines| lines.last())
            .filter(|line| line.role == "assistant")
            .map(|line| line.body.clone())
        else {
            return;
        };
        let _ = self.store.update_last_message(&id, "assistant", &body);
    }

    pub fn view_context(&self) -> String {
        if let Some(report) = self.open_report() {
            return format!(
                "The user is in the chat for report {} — {}. Answer only from that report's content. Do not use brain memories or other reports. If this report does not contain the answer, say so.",
                report.id, report.title
            );
        }
        let mut ctx = match self
            .chat_case
            .as_ref()
            .and_then(|id| self.cases.iter().find(|case| &case.id == id))
        {
            Some(case) => format!(
                "The user is talking about case {} — {}. Answer this investigation.\n{}",
                case.id,
                case.title,
                self.report_inventory()
            ),
            None => format!(
                "The user is on the case desk. This desk starts new case queries and discusses every report listed beside it.\n{}",
                self.report_inventory()
            ),
        };
        if let Some(widget) = self.widget() {
            ctx.push_str(&format!(
                "\nA {} widget is open beside the case desk for configuration or data. It is not a separate chat.",
                widget.title()
            ));
            ctx.push('\n');
            ctx.push_str(&self.widget_snapshot(widget));
        }
        ctx
    }

    pub fn report_rows(&self) -> Vec<ReportRow> {
        let mut rows: Vec<ReportRow> = self
            .pending_reports
            .iter()
            .rev()
            .map(|pending| ReportRow::Pending {
                case_id: pending.case_id.clone(),
                title: pending.title.clone(),
                failed: pending.failed.clone(),
            })
            .collect();
        rows.extend(self.reports.iter().cloned().map(ReportRow::Completed));
        rows
    }

    fn report_inventory(&self) -> String {
        let rows = self.report_rows();
        if rows.is_empty() {
            return "No reports yet.".into();
        }
        let mut out = String::from("Reports beside the desk:\n");
        for row in rows.iter().take(12) {
            match row {
                ReportRow::Pending {
                    title,
                    failed: Some(_),
                    ..
                } => {
                    out.push_str(&format!("- failed: {title}\n"));
                }
                ReportRow::Pending { title, .. } => {
                    out.push_str(&format!("- pending: {title}\n"));
                }
                ReportRow::Completed(report) => {
                    out.push_str(&format!(
                        "- completed: {} ({})\n",
                        report.title, report.path
                    ));
                }
            }
        }
        out
    }

    fn widget_snapshot(&self, widget: ModuleId) -> String {
        match widget {
            ModuleId::Hardware => self.hardware.one_line(),
            ModuleId::Osint => format!(
                "Facts {}, Web {}, News {}, Domain {}, Social {}, Identity {}, {} extra sources.",
                on_off(self.settings.facts),
                on_off(self.settings.web),
                on_off(self.settings.news),
                on_off(self.settings.domain),
                on_off(self.settings.social),
                on_off(self.settings.identity),
                self.settings.sources.len()
            ),
            ModuleId::Providers => format!(
                "Connection: {}. Voice: {}. Writer: {}. Tool caller: {}.",
                provider_label(self.auth.text.as_ref()),
                provider_label(self.auth.voice.as_ref()),
                self.role_label(&self.settings.writer_model),
                self.role_label(&self.settings.tool_model)
            ),
            ModuleId::Brain => format!("{} memories stored.", self.memories.len()),
            ModuleId::Gmail => self
                .auth
                .gmail
                .as_ref()
                .map(|gmail| format!("Gmail account {}.", gmail.email))
                .unwrap_or_else(|| "Gmail is not connected.".into()),
            ModuleId::Reports => format!("{} reports on disk.", self.reports.len()),
            ModuleId::Log => "The System log is open. Do not repeat its lines.".into(),
            ModuleId::System => "System is open.".into(),
            ModuleId::Settings => format!(
                "SearXNG: {}. Reports: {}.",
                if self.settings.searx_url.is_empty() {
                    "public fallback"
                } else {
                    self.settings.searx_url.as_str()
                },
                report_dir(&self.settings).display()
            ),
            ModuleId::Cases => String::new(),
        }
    }

    pub fn filtered_modules(&self) -> Vec<ModuleId> {
        let q = self.modal_query.trim().to_lowercase();
        ModuleId::all()
            .into_iter()
            .filter(|m| {
                q.is_empty()
                    || m.title().to_lowercase().contains(&q)
                    || m.blurb().to_lowercase().contains(&q)
            })
            .collect()
    }

    pub fn tick(&mut self) {
        self.spinner = self.spinner.wrapping_add(1);
        self.tick_n = self.tick_n.wrapping_add(1);
        if self.tick_n % 5 != 0 {
            return;
        }
        self.sys.refresh_memory();
        self.sys.refresh_cpu_usage();
        let cpu = self.sys.global_cpu_usage();
        self.cpu_now = cpu;
        self.cpu_hist
            .push_back(cpu.round().clamp(0.0, 100.0) as u64);
        if self.cpu_hist.len() > 48 {
            self.cpu_hist.pop_front();
        }
        let total = self.sys.total_memory() as f64 / 1_073_741_824.0;
        let avail = self.sys.available_memory() as f64 / 1_073_741_824.0;
        if total > 0.0 {
            self.hardware.total_ram_gb = (total * 10.0).round() / 10.0;
            self.hardware.available_ram_gb = (avail * 10.0).round() / 10.0;
            self.hardware.cpu_usage = cpu;
            if self.hardware.logical_cores == 0 {
                self.hardware.logical_cores = self.sys.cpus().len();
            }
        }
    }

    pub fn spawn_hardware(&self, fresh: bool) {
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let profile =
                tokio::task::spawn_blocking(move || hardware::profile_cached(fresh)).await;
            if let Ok(profile) = profile {
                let _ = tx.send(AppMsg::Hardware(profile));
            }
        });
    }

    pub fn on_msg(&mut self, msg: AppMsg) {
        match msg {
            AppMsg::Turn(ev) => self.on_turn(ev),
            AppMsg::Hardware(profile) => {
                let cpu = self.cpu_now;
                let hist_cores = self.hardware.logical_cores;
                self.hardware = profile;
                if self.hardware.cpu_usage == 0.0 {
                    self.hardware.cpu_usage = cpu;
                }
                if self.hardware.logical_cores == 0 {
                    self.hardware.logical_cores = hist_cores;
                }
                if let Some(err) = self.hardware.gpu_error.clone() {
                    self.log_event("system", &format!("hardware: {err}"));
                } else {
                    self.log_event("system", "hardware profile refreshed");
                }
            }
            AppMsg::Note(text) => self.log_event("api", &text),
            AppMsg::Search { query, result } => self.finish_search(query, result),
            AppMsg::ModelList(result) => match result {
                Ok(models) => {
                    self.log_event("api", &format!("models listed: {}", models.len()));
                    self.remote_models = models.iter().map(|model| model.id.clone()).collect();
                    self.catalog = models;
                    self.free_loading = false;
                }
                Err(err) => {
                    self.free_loading = false;
                    self.log_event("api", &format!("models failed: {err}"));
                }
            },
            AppMsg::Models(result) => match result {
                Ok(names) => {
                    let shown = if names.is_empty() {
                        "endpoint answered, no model ids".to_string()
                    } else {
                        format!(
                            "{} models, first: {}",
                            names.len(),
                            names.iter().take(6).cloned().collect::<Vec<_>>().join(", ")
                        )
                    };
                    self.status = "provider ok".into();
                    self.log_event("api", &shown);
                }
                Err(err) => {
                    self.status = "provider error".into();
                    self.log_event("api", &format!("provider: {err}"));
                }
            },
            AppMsg::Voice(result) => match result {
                Ok(text) => {
                    self.prompt = text;
                    self.cursor = self.prompt.chars().count();
                    self.status = "transcript ready".into();
                    self.focus = Focus::Prompt;
                }
                Err(err) => {
                    self.status = "voice error".into();
                    self.log_event("api", &format!("voice: {err}"));
                }
            },
            AppMsg::Research { case_id, result } => self.finish_research(case_id, result),
            AppMsg::Insight { report_id, fact } => self.store_report_insight(report_id, fact),
            AppMsg::GmailTest(result) => match result {
                Ok(text) => {
                    self.status = "gmail ok".into();
                    self.log_event("api", &text);
                }
                Err(err) => {
                    self.status = "gmail error".into();
                    self.log_event("api", &format!("gmail: {err}"));
                }
            },
        }
    }

    fn on_turn(&mut self, ev: TurnEvent) {
        match ev {
            TurnEvent::Status(text) => self.status = text,
            TurnEvent::Delta(text) => {
                let id = self.session_id();
                let lines = self.transcripts.entry(id).or_default();
                if let Some(last) = lines.last_mut() {
                    if last.role == "assistant" {
                        last.body.push_str(&text);
                    }
                }
            }
            TurnEvent::Note(text) => self.log_event(note_kind(&text), &text),
            TurnEvent::Report(meta) => {
                let _ = self.store.add_report(&meta);
                self.log_event("task", &format!("report {}", meta.path));
                let _ = self.reload_lists();
                self.after_report_filed(&meta.id);
            }
            TurnEvent::Memory(memory) => {
                if let Ok(saved) = self.store.add_memory(&memory.text) {
                    self.memories.insert(0, saved);
                    self.log_event("system", "brain updated");
                }
            }
            TurnEvent::Done(text) => {
                self.running = false;
                self.status = "ready".into();
                self.replace_last_assistant(&text);
                self.capture_report_insight(&text);
            }
            TurnEvent::Failed(err) => {
                self.running = false;
                self.status = "error".into();
                self.log_event("api", &err);
                self.replace_last_assistant(
                    "Could not finish that request. The detail is in the System log.",
                );
            }
        }
    }

    fn finish_search(&mut self, query: String, result: Result<Vec<SearchHit>, String>) {
        self.running = false;
        match result {
            Ok(hits) => {
                let title = query.trim().chars().take(72).collect::<String>();
                let md = report::source_pack(
                    if title.is_empty() { "Search" } else { &title },
                    self.case_id().as_deref(),
                    &query,
                    &hits,
                );
                match report::write_report(
                    &report_dir(&self.settings),
                    &title,
                    self.case_id().as_deref(),
                    &md,
                ) {
                    Ok(meta) => {
                        let _ = self.store.add_report(&meta);
                        let _ = self.reload_lists();
                        self.push_line(
                            "assistant",
                            &format!("{}\n\nReport: {}", summarize_hits(&hits), meta.path),
                        );
                        self.log_event("search", &format!("{} hits", hits.len()));
                    }
                    Err(err) => {
                        self.log_event("task", &format!("report write failed: {err}"));
                        self.replace_last_assistant(
                            "The report could not be written. The detail is in the System log.",
                        );
                    }
                }
            }
            Err(err) => {
                self.log_event("search", &format!("search failed: {err}"));
                self.replace_last_assistant("The search failed. The detail is in the System log.");
            }
        }
        self.status = "ready".into();
    }

    fn case_id(&self) -> Option<String> {
        self.chat_case.clone()
    }

    pub fn on_event(&mut self, ev: Event) -> bool {
        match ev {
            Event::Key(key) => self.on_key(key),
            Event::Mouse(mouse) => {
                match mouse.kind {
                    MouseEventKind::Down(crossterm::event::MouseButton::Left) => {
                        self.click(mouse.column, mouse.row);
                    }
                    MouseEventKind::ScrollUp => self.scroll_chat_at(mouse.column, mouse.row, 3),
                    MouseEventKind::ScrollDown => self.scroll_chat_at(mouse.column, mouse.row, -3),
                    _ => {}
                }
                false
            }
            _ => false,
        }
    }

    fn click(&mut self, x: u16, y: u16) {
        if self.free_picker
            || self.model_picker
            || self.scope.is_some()
            || self.confirm_query.is_some()
            || self.confirm_report.is_some()
            || self.confirm_delete_report.is_some()
            || self.brain_card
            || self.modal
            || self.help
        {
            return;
        }
        let pos = ratatui::layout::Position { x, y };
        if self.report_area.contains(pos) {
            self.focus = Focus::Reports;
            let rel = y.saturating_sub(self.report_area.y + 1) as usize;
            if let Some(Some(index)) = self.report_line_index.get(rel).copied() {
                self.report_sel = index;
                self.ask_open_report();
            }
            return;
        }
        if let Some(index) = self.case_tab_hits.iter().position(|tab| tab.contains(pos)) {
            self.select_case_page(CasePage::all()[index]);
            return;
        }
        if let Some(index) = self
            .provider_tab_hits
            .iter()
            .position(|tab| tab.contains(pos))
        {
            self.select_provider_page(ProviderPage::all()[index]);
            return;
        }
        if let Some(index) = self
            .system_tab_hits
            .iter()
            .position(|tab| tab.contains(pos))
        {
            self.select_system_page(SystemPage::all()[index]);
            return;
        }
        if self.canvas_area.contains(pos) {
            self.focus = Focus::Canvas;
            return;
        }
        if self.launcher_area.contains(pos) {
            let rel = y.saturating_sub(self.launcher_area.y + 1) as usize;
            if rel < ModuleId::all().len() {
                self.launcher_sel = rel;
                self.open_module(ModuleId::all()[rel]);
            }
        }
    }

    fn select_case_page(&mut self, page: CasePage) {
        self.module = Some(ModuleId::Cases);
        self.case_page = page;
        self.field_sel = 0;
        self.editing = false;
        if page == CasePage::Closed {
            if self.chat_report.is_some() {
                self.persist_visible_chat();
            }
            self.chat_report = None;
            self.fields.clear();
            self.focus = Focus::Prompt;
            self.scroll_back = 0;
            self.load_transcript("desk");
            return;
        }
        if page == CasePage::Network {
            self.fields.clear();
            self.focus = Focus::Graph;
            self.tna_find = None;
            self.tna_sel = 0;
            self.tna_side_scroll = 0;
            self.tna_view = TnaView::Graph;
            self.tna_expanded.clear();
            self.tna_focus_id = None;
            self.ensure_tna_snapshot(false);
            return;
        }
        self.focus = Focus::Canvas;
        self.load_group_fields();
    }

    fn select_system_page(&mut self, page: SystemPage) {
        self.module = Some(ModuleId::System);
        self.system_page = page;
        self.field_sel = 0;
        self.editing = false;
        self.focus = Focus::Canvas;
        self.load_group_fields();
        if page == SystemPage::Hardware {
            self.spawn_hardware(false);
        }
    }

    fn select_provider_page(&mut self, page: ProviderPage) {
        self.module = Some(ModuleId::Providers);
        self.provider_page = page;
        self.field_sel = 0;
        self.editing = false;
        self.focus = Focus::Canvas;
        self.load_group_fields();
    }

    fn on_key(&mut self, key: KeyEvent) -> bool {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if self.scope.is_some() && !(ctrl && key.code == KeyCode::Char('c')) {
            return self.on_scope_key(key);
        }
        if self.confirm_query.is_some() && !(ctrl && key.code == KeyCode::Char('c')) {
            return self.on_confirm_key(key);
        }
        if self.confirm_delete_report.is_some() && !(ctrl && key.code == KeyCode::Char('c')) {
            return self.on_delete_report_key(key);
        }
        if self.confirm_report.is_some() && !(ctrl && key.code == KeyCode::Char('c')) {
            return self.on_report_confirm_key(key);
        }
        if self.brain_card && !(ctrl && key.code == KeyCode::Char('c')) {
            return self.on_brain_card_key(key);
        }
        if self.help {
            self.help = false;
            return false;
        }
        if self.free_picker && !(ctrl && key.code == KeyCode::Char('c')) {
            return self.on_free_key(key);
        }
        if self.model_picker && !(ctrl && key.code == KeyCode::Char('c')) {
            return self.on_model_key(key);
        }
        if ctrl && key.code == KeyCode::Char('c') {
            return self.cancel_or_quit();
        }
        if self.modal {
            return self.on_modal_key(key);
        }
        if self.on_case_desk() && !self.editing {
            if key.code == KeyCode::Char('+') {
                self.start_case_from_prompt();
                return false;
            }
            if key.code == KeyCode::Char('x')
                && (self.focus != Focus::Prompt || self.prompt.is_empty())
            {
                self.delete_highlighted_case();
                return false;
            }
        }
        if ctrl && key.code == KeyCode::Char('p') {
            self.modal = true;
            self.modal_query.clear();
            self.modal_sel = 0;
            return false;
        }
        if ctrl && key.code == KeyCode::Char('m') && !self.editing {
            self.open_model_picker();
            return false;
        }
        if ctrl && key.code == KeyCode::Char('r') && self.focus == Focus::Prompt {
            self.record_voice();
            return false;
        }
        if self.editing {
            return self.on_field_key(key);
        }
        if self.chat_is_on_screen() {
            match key.code {
                KeyCode::PageUp => {
                    self.scroll_chat(8);
                    return false;
                }
                KeyCode::PageDown => {
                    self.scroll_chat(-8);
                    return false;
                }
                KeyCode::End => {
                    self.scroll_back = 0;
                    return false;
                }
                _ => {}
            }
        }
        match key.code {
            KeyCode::Tab => {
                self.focus = self.next_focus();
                false
            }
            KeyCode::Esc => {
                self.on_esc();
                false
            }
            KeyCode::Char('?') if self.focus != Focus::Prompt && self.prompt.is_empty() => {
                self.help = true;
                false
            }
            _ if self.focus == Focus::Prompt => self.on_prompt_key(key),
            _ if self.focus == Focus::Launcher => self.on_launcher_key(key),
            _ if self.focus == Focus::Reports => self.on_reports_key(key),
            _ if self.focus == Focus::Graph || self.focus == Focus::TnaSide => self.on_tna_key(key),
            _ => self.on_canvas_key(key),
        }
    }

    fn cancel_or_quit(&mut self) -> bool {
        if self.running {
            self.cancel.store(true, Ordering::Relaxed);
            self.status = "cancelling".into();
            false
        } else if !self.prompt.is_empty() && self.focus == Focus::Prompt {
            self.prompt.clear();
            self.cursor = 0;
            false
        } else {
            self.quit = true;
            true
        }
    }

    fn chat_is_on_screen(&self) -> bool {
        self.on_case_desk() && self.case_page == CasePage::Closed
    }

    fn scroll_chat(&mut self, delta: isize) {
        if !self.chat_is_on_screen() {
            return;
        }
        if delta > 0 {
            self.scroll_back = self.scroll_back.saturating_add(delta as usize);
        } else {
            self.scroll_back = self.scroll_back.saturating_sub((-delta) as usize);
        }
    }

    fn scroll_chat_at(&mut self, x: u16, y: u16, delta: isize) {
        let pos = ratatui::layout::Position { x, y };
        if self.focus == Focus::Canvas || self.canvas_area.contains(pos) {
            self.scroll_chat(delta);
        }
    }

    fn next_focus(&self) -> Focus {
        let reports = self.on_case_desk() && self.case_page == CasePage::Closed;
        let network = self.on_case_desk() && self.case_page == CasePage::Network;
        match self.focus {
            Focus::Launcher => {
                if network {
                    Focus::Graph
                } else {
                    Focus::Canvas
                }
            }
            Focus::Graph => Focus::TnaSide,
            Focus::TnaSide => Focus::Prompt,
            Focus::Canvas if reports => Focus::Reports,
            Focus::Canvas | Focus::Reports => Focus::Prompt,
            Focus::Prompt => Focus::Launcher,
        }
    }

    fn on_reports_key(&mut self, key: KeyEvent) -> bool {
        let n = self.report_rows().len();
        match key.code {
            KeyCode::Up | KeyCode::Char('k') if n > 0 => {
                self.report_sel = (self.report_sel + n - 1) % n;
            }
            KeyCode::Down | KeyCode::Char('j') if n > 0 => {
                self.report_sel = (self.report_sel + 1) % n;
            }
            KeyCode::Enter => self.ask_open_report(),
            KeyCode::Left => {
                self.cycle_group_page(-1);
                if self.case_page != CasePage::Closed {
                    self.focus = Focus::Canvas;
                }
            }
            KeyCode::Right => {
                self.cycle_group_page(1);
                if self.case_page != CasePage::Closed {
                    self.focus = Focus::Canvas;
                }
            }
            KeyCode::Esc => self.on_esc(),
            _ => {}
        }
        false
    }

    fn ask_open_report(&mut self) {
        let Some(row) = self.report_rows().get(self.report_sel).cloned() else {
            self.status = "no report selected".into();
            return;
        };
        match row {
            ReportRow::Completed(report) => {
                self.confirm_report = Some(report.id);
                self.confirm_report_sel = 0;
            }
            ReportRow::Pending { failed: None, .. } => {
                self.status = "that report is still pending".into();
            }
            ReportRow::Pending { .. } => {
                self.status = "that report has no file to open".into();
            }
        }
    }

    fn on_report_confirm_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                self.confirm_report_sel = (self.confirm_report_sel + 2) % 3;
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.confirm_report_sel = (self.confirm_report_sel + 1) % 3;
            }
            KeyCode::Esc => {
                self.confirm_report = None;
            }
            KeyCode::Enter => self.run_report_confirm(),
            _ => {}
        }
        false
    }

    fn run_report_confirm(&mut self) {
        let Some(id) = self.confirm_report.take() else {
            return;
        };
        match self.confirm_report_sel {
            0 => self.open_report_chat(&id),
            1 => {
                self.confirm_delete_report = Some(id);
                self.confirm_delete_sel = 1;
            }
            _ => {}
        }
    }

    fn on_delete_report_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') | KeyCode::Down | KeyCode::Char('j') => {
                self.confirm_delete_sel = if self.confirm_delete_sel == 0 { 1 } else { 0 };
            }
            KeyCode::Esc => {
                self.confirm_delete_report = None;
            }
            KeyCode::Enter => {
                let delete = self.confirm_delete_sel == 0;
                if let Some(id) = self.confirm_delete_report.take() {
                    if delete {
                        self.delete_report_and_memories(&id);
                    }
                }
            }
            _ => {}
        }
        false
    }

    fn delete_report_and_memories(&mut self, id: &str) {
        let path = self
            .reports
            .iter()
            .find(|report| report.id == id)
            .map(|report| report.path.clone());
        if let Some(path) = path {
            if let Err(err) = std::fs::remove_file(&path) {
                if err.kind() != std::io::ErrorKind::NotFound {
                    self.status = "could not delete the report file".into();
                    self.log_event("task", &format!("delete report file: {err}"));
                    return;
                }
            }
        }
        if self.store.delete_report_bundle(id).is_err() {
            self.status = "could not delete the report".into();
            return;
        }
        self.transcripts.remove(&format!("report:{id}"));
        if self.chat_report.as_deref() == Some(id) {
            self.chat_report = None;
            self.load_transcript("desk");
            self.scroll_back = 0;
        }
        self.after_report_deleted(id);
        let _ = self.reload_lists();
        self.status = "report deleted".into();
    }


    pub fn tna_snapshot(&self) -> Option<&TnaSnapshot> {
        if self.chat_report.is_some() {
            self.tna_report.as_ref()
        } else {
            self.tna_desk.as_ref()
        }
    }

    /// Find-filter over raw node indices (Outline / Table / legacy).
    pub fn tna_visible_nodes(&self) -> Vec<usize> {
        let Some(snap) = self.tna_snapshot() else {
            return Vec::new();
        };
        let q = self.tna_find_query();
        snap.nodes
            .iter()
            .enumerate()
            .filter(|(_, n)| Self::tna_node_matches(n, &q))
            .map(|(i, _)| i)
            .collect()
    }

    fn tna_find_query(&self) -> String {
        self.tna_find
            .as_deref()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase()
    }

    fn tna_node_matches(n: &TnaNode, q: &str) -> bool {
        q.is_empty()
            || n.label.to_ascii_lowercase().contains(q)
            || n.id.to_ascii_lowercase().contains(q)
    }

    fn tna_adjacency(snap: &TnaSnapshot) -> HashMap<String, Vec<String>> {
        let mut adj: HashMap<String, Vec<String>> = HashMap::new();
        for e in &snap.edges {
            adj.entry(e.from.clone()).or_default().push(e.to.clone());
            adj.entry(e.to.clone()).or_default().push(e.from.clone());
        }
        adj
    }

    fn tna_hops_from(
        focus_id: &str,
        adj: &HashMap<String, Vec<String>>,
        max_hops: u32,
    ) -> HashSet<String> {
        let mut out = HashSet::new();
        out.insert(focus_id.to_string());
        let mut frontier = vec![focus_id.to_string()];
        for _ in 0..max_hops {
            let mut next = Vec::new();
            for id in &frontier {
                for nb in adj.get(id).into_iter().flatten() {
                    if out.insert(nb.clone()) {
                        next.push(nb.clone());
                    }
                }
            }
            frontier = next;
            if frontier.is_empty() {
                break;
            }
        }
        out
    }

    /// Graph canvas items: egocentric 1–2 hops with hub collapse to ▣×N.
    pub fn tna_display_nodes(&self) -> Vec<TnaDisplayItem> {
        let Some(snap) = self.tna_snapshot() else {
            return Vec::new();
        };
        if snap.nodes.is_empty() {
            return Vec::new();
        }
        let q = self.tna_find_query();
        let adj = Self::tna_adjacency(snap);
        let id_to_idx: HashMap<&str, usize> = snap
            .nodes
            .iter()
            .enumerate()
            .map(|(i, n)| (n.id.as_str(), i))
            .collect();

        let focus_id = self
            .tna_focus_id
            .clone()
            .filter(|id| snap.nodes.iter().any(|n| n.id == *id))
            .unwrap_or_else(|| snap.nodes[0].id.clone());

        let ego1 = Self::tna_hops_from(&focus_id, &adj, 1);
        let ego2 = Self::tna_hops_from(&focus_id, &adj, 2);
        let threshold = tna_hub_threshold(&snap.nodes);

        let mut collapsed: HashSet<String> = HashSet::new();
        let mut supers: Vec<TnaDisplayItem> = Vec::new();

        for node in &snap.nodes {
            if !ego1.contains(&node.id) {
                continue;
            }
            if node.degree < threshold || self.tna_expanded.contains(&node.id) {
                continue;
            }
            let neighbors = adj.get(&node.id).map(|v| v.as_slice()).unwrap_or(&[]);
            let far: Vec<&String> = neighbors
                .iter()
                .filter(|n| !ego1.contains(*n))
                .collect();
            if far.is_empty() {
                continue;
            }
            for f in &far {
                collapsed.insert((*f).clone());
            }
            // Offset supernode slightly from the hub so glyphs don't stack.
            let (x, y) = (node.x + 0.04, (node.y + 0.03).min(1.0));
            supers.push(TnaDisplayItem::Super {
                hub_id: node.id.clone(),
                count: far.len(),
                x,
                y,
            });
        }

        let mut items: Vec<TnaDisplayItem> = Vec::new();
        let mut order: Vec<usize> = ego2
            .iter()
            .filter_map(|id| id_to_idx.get(id.as_str()).copied())
            .collect();
        order.sort_unstable();
        for idx in order {
            let n = &snap.nodes[idx];
            if collapsed.contains(&n.id) {
                continue;
            }
            if !Self::tna_node_matches(n, &q) {
                continue;
            }
            items.push(TnaDisplayItem::Real { idx });
        }

        for s in supers {
            if let TnaDisplayItem::Super {
                ref hub_id,
                count,
                x,
                y,
            } = s
            {
                let hub_ok = snap
                    .nodes
                    .iter()
                    .find(|n| n.id == *hub_id)
                    .map(|n| Self::tna_node_matches(n, &q))
                    .unwrap_or(false);
                let far_ok = if q.is_empty() {
                    true
                } else {
                    adj.get(hub_id)
                        .into_iter()
                        .flatten()
                        .filter(|nid| !ego1.contains(*nid))
                        .filter_map(|nid| id_to_idx.get(nid.as_str()).copied())
                        .any(|i| Self::tna_node_matches(&snap.nodes[i], &q))
                };
                if q.is_empty() || hub_ok || far_ok {
                    items.push(TnaDisplayItem::Super {
                        hub_id: hub_id.clone(),
                        count,
                        x,
                        y,
                    });
                }
            }
        }
        items
    }

    pub fn tna_selection_list(&self) -> Vec<TnaDisplayItem> {
        match self.tna_view {
            TnaView::Graph => self.tna_display_nodes(),
            TnaView::Outline | TnaView::Table => self
                .tna_visible_nodes()
                .into_iter()
                .map(|idx| TnaDisplayItem::Real { idx })
                .collect(),
        }
    }

    pub fn tna_selected_item(&self) -> Option<TnaDisplayItem> {
        self.tna_selection_list().get(self.tna_sel).cloned()
    }

    pub fn tna_selected_real_idx(&self) -> Option<usize> {
        match self.tna_selected_item()? {
            TnaDisplayItem::Real { idx } => Some(idx),
            TnaDisplayItem::Super { .. } => None,
        }
    }

    pub fn tna_selected_super_hub(&self) -> Option<String> {
        match self.tna_selected_item()? {
            TnaDisplayItem::Super { hub_id, .. } => Some(hub_id),
            TnaDisplayItem::Real { .. } => None,
        }
    }

    /// Update egocentric focus and re-pin selection index in the new display list.
    fn pin_tna_selection(&mut self, item: Option<TnaDisplayItem>) {
        let Some(item) = item else {
            return;
        };
        match &item {
            TnaDisplayItem::Real { idx } => {
                if let Some(snap) = self.tna_snapshot() {
                    if let Some(n) = snap.nodes.get(*idx) {
                        self.tna_focus_id = Some(n.id.clone());
                    }
                }
            }
            TnaDisplayItem::Super { hub_id, .. } => {
                self.tna_focus_id = Some(hub_id.clone());
            }
        }
        if self.tna_view != TnaView::Graph {
            return;
        }
        let items = self.tna_display_nodes();
        let pos = items.iter().position(|it| match (&item, it) {
            (TnaDisplayItem::Real { idx: a }, TnaDisplayItem::Real { idx: b }) => a == b,
            (
                TnaDisplayItem::Super { hub_id: a, .. },
                TnaDisplayItem::Super { hub_id: b, .. },
            ) => a == b,
            _ => false,
        });
        if let Some(pos) = pos {
            self.tna_sel = pos;
        } else {
            let n = items.len();
            self.tna_sel = if n == 0 { 0 } else { self.tna_sel.min(n - 1) };
        }
    }

    pub fn tna_selectable_count(&self) -> usize {
        match self.tna_view {
            TnaView::Graph => self.tna_display_nodes().len(),
            TnaView::Outline | TnaView::Table => self.tna_visible_nodes().len(),
        }
    }

    fn clamp_tna_sel(&mut self) {
        let n = self.tna_selectable_count();
        if n == 0 {
            self.tna_sel = 0;
        } else if self.tna_sel >= n {
            self.tna_sel = n - 1;
        }
        if self.tna_focus_id.is_none() {
            if let Some(TnaDisplayItem::Real { idx }) = self.tna_selected_item() {
                if let Some(snap) = self.tna_snapshot() {
                    if let Some(n) = snap.nodes.get(idx) {
                        self.tna_focus_id = Some(n.id.clone());
                    }
                }
            } else if let Some(TnaDisplayItem::Super { hub_id, .. }) = self.tna_selected_item() {
                self.tna_focus_id = Some(hub_id);
            }
        }
    }

    /// Pure helper: which node indices stay visible under ego + collapse.
    pub fn tna_ego_visible_indices(
        snap: &TnaSnapshot,
        focus_id: &str,
        expanded: &HashSet<String>,
    ) -> (Vec<usize>, Vec<(String, usize)>) {
        if snap.nodes.is_empty() {
            return (Vec::new(), Vec::new());
        }
        let adj = Self::tna_adjacency(snap);
        let id_to_idx: HashMap<&str, usize> = snap
            .nodes
            .iter()
            .enumerate()
            .map(|(i, n)| (n.id.as_str(), i))
            .collect();
        let ego1 = Self::tna_hops_from(focus_id, &adj, 1);
        let ego2 = Self::tna_hops_from(focus_id, &adj, 2);
        let threshold = tna_hub_threshold(&snap.nodes);
        let mut collapsed: HashSet<String> = HashSet::new();
        let mut supers: Vec<(String, usize)> = Vec::new();
        for node in &snap.nodes {
            if !ego1.contains(&node.id) {
                continue;
            }
            if node.degree < threshold || expanded.contains(&node.id) {
                continue;
            }
            let neighbors = adj.get(&node.id).map(|v| v.as_slice()).unwrap_or(&[]);
            let far: Vec<&String> = neighbors
                .iter()
                .filter(|n| !ego1.contains(*n))
                .collect();
            if far.is_empty() {
                continue;
            }
            for f in &far {
                collapsed.insert((*f).clone());
            }
            supers.push((node.id.clone(), far.len()));
        }
        let mut idxs: Vec<usize> = ego2
            .iter()
            .filter(|id| !collapsed.contains(*id))
            .filter_map(|id| id_to_idx.get(id.as_str()).copied())
            .collect();
        idxs.sort_unstable();
        (idxs, supers)
    }

    fn ensure_tna_snapshot(&mut self, force: bool) {
        if self.tna_rebuilding {
            return;
        }
        if self.chat_report.is_some() {
            if force || self.tna_report.is_none() {
                self.spawn_tna_rebuild(true);
            }
        } else if force || self.tna_desk.is_none() {
            self.spawn_tna_rebuild(false);
        }
    }

    fn spawn_tna_rebuild(&mut self, targeted: bool) {
        self.tna_rebuilding = true;
        if targeted {
            if let Some(id) = self.chat_report.clone() {
                match tna::rebuild_for_report(&self.store, &id) {
                    Ok(snap) => self.tna_report = Some(snap),
                    Err(err) => self.log_event("task", &format!("tna rebuild failed: {err}")),
                }
            }
        } else {
            match tna::rebuild_collection(&self.store) {
                Ok(snap) => self.tna_desk = Some(snap),
                Err(err) => self.log_event("task", &format!("tna rebuild failed: {err}")),
            }
        }
        self.tna_rebuilding = false;
        self.clamp_tna_sel();
    }

    fn after_report_filed(&mut self, report_id: &str) {
        let _ = tna::rebuild_after_file(&self.store, report_id);
        if let Ok(Some(snap)) = self.store.get_tna_graph(desk_key()) {
            self.tna_desk = Some(snap);
        }
        if let Ok(Some(snap)) = self.store.get_tna_graph(&report_key(report_id)) {
            if self.chat_report.as_deref() == Some(report_id) {
                self.tna_report = Some(snap);
            }
        }
        if self.case_page == CasePage::Network {
            self.ensure_tna_snapshot(true);
        }
    }

    fn after_report_deleted(&mut self, report_id: &str) {
        let _ = tna::rebuild_after_delete(&self.store, report_id);
        self.tna_report = None;
        if let Ok(Some(snap)) = self.store.get_tna_graph(desk_key()) {
            self.tna_desk = Some(snap);
        } else {
            self.tna_desk = None;
        }
    }

    fn on_tna_key(&mut self, key: KeyEvent) -> bool {
        if self.tna_find.is_some() {
            match key.code {
                KeyCode::Esc => {
                    self.tna_find = None;
                }
                KeyCode::Backspace => {
                    if let Some(q) = self.tna_find.as_mut() {
                        q.pop();
                    }
                }
                KeyCode::Enter => {
                    // keep filter active but stop editing
                }
                KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    if let Some(q) = self.tna_find.as_mut() {
                        q.push(c);
                    }
                }
                _ => {}
            }
            self.clamp_tna_sel();
            return false;
        }
        match key.code {
            KeyCode::Char('/') => {
                self.tna_find = Some(String::new());
                self.focus = Focus::Graph;
            }
            KeyCode::Char('v') => {
                self.tna_view = self.tna_view.next();
                self.tna_sel = 0;
                self.clamp_tna_sel();
            }
            KeyCode::Enter => self.tna_activate_selection(),
            KeyCode::Char('h') | KeyCode::Left => {
                if self.tna_view == TnaView::Graph {
                    self.walk_tna_node(-1, 0);
                }
            }
            KeyCode::Char('l') | KeyCode::Right => {
                if self.tna_view == TnaView::Graph {
                    self.walk_tna_node(1, 0);
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                if self.focus == Focus::TnaSide {
                    self.tna_side_scroll = self.tna_side_scroll.saturating_sub(1);
                } else if self.tna_view == TnaView::Graph {
                    self.walk_tna_node(0, -1);
                } else {
                    self.tna_step_sel(-1);
                }
            }
            KeyCode::Char('j') | KeyCode::Down => {
                if self.focus == Focus::TnaSide {
                    self.tna_side_scroll = self.tna_side_scroll.saturating_add(1);
                } else if self.tna_view == TnaView::Graph {
                    self.walk_tna_node(0, 1);
                } else {
                    self.tna_step_sel(1);
                }
            }
            KeyCode::Tab => {
                self.focus = if self.focus == Focus::Graph {
                    Focus::TnaSide
                } else {
                    Focus::Graph
                };
            }
            _ => {}
        }
        false
    }

    fn tna_step_sel(&mut self, delta: i32) {
        let n = self.tna_selectable_count();
        if n == 0 {
            self.tna_sel = 0;
            return;
        }
        let cur = self.tna_sel as i32;
        let next = (cur + delta).rem_euclid(n as i32) as usize;
        self.tna_sel = next;
        // Outline/Table: keep focus id in sync for when user switches back to Graph.
        if let Some(TnaDisplayItem::Real { idx }) = self.tna_selected_item() {
            if let Some(snap) = self.tna_snapshot() {
                if let Some(n) = snap.nodes.get(idx) {
                    self.tna_focus_id = Some(n.id.clone());
                }
            }
        }
    }

    fn tna_activate_selection(&mut self) {
        if self.tna_view != TnaView::Graph {
            return;
        }
        let item = match self.tna_selected_item() {
            Some(i) => i,
            None => return,
        };
        match item {
            TnaDisplayItem::Super { hub_id, .. } => {
                self.tna_expanded.insert(hub_id.clone());
                let hub_idx = self
                    .tna_snapshot()
                    .and_then(|s| s.nodes.iter().position(|n| n.id == hub_id));
                if let Some(idx) = hub_idx {
                    self.pin_tna_selection(Some(TnaDisplayItem::Real { idx }));
                } else {
                    self.clamp_tna_sel();
                }
            }
            TnaDisplayItem::Real { idx } => {
                let Some(snap) = self.tna_snapshot() else {
                    return;
                };
                let id = snap.nodes.get(idx).map(|n| n.id.clone());
                let degree = snap.nodes.get(idx).map(|n| n.degree).unwrap_or(0);
                let threshold = tna_hub_threshold(&snap.nodes);
                if let Some(id) = id {
                    if degree >= threshold && self.tna_expanded.contains(&id) {
                        self.tna_expanded.remove(&id);
                        self.pin_tna_selection(Some(TnaDisplayItem::Real { idx }));
                    }
                }
            }
        }
    }

    fn walk_tna_node(&mut self, dx: i32, dy: i32) {
        let items = self.tna_display_nodes();
        if items.is_empty() {
            return;
        }
        let snap = match self.tna_snapshot() {
            Some(s) => s,
            None => return,
        };
        let cur_i = self.tna_sel.min(items.len() - 1);
        let (cx, cy) = match &items[cur_i] {
            TnaDisplayItem::Real { idx } => (snap.nodes[*idx].x, snap.nodes[*idx].y),
            TnaDisplayItem::Super { x, y, .. } => (*x, *y),
        };
        let mut best: Option<(usize, f64)> = None;
        for (vis_i, item) in items.iter().enumerate() {
            if vis_i == cur_i {
                continue;
            }
            let (nx, ny) = match item {
                TnaDisplayItem::Real { idx } => (snap.nodes[*idx].x, snap.nodes[*idx].y),
                TnaDisplayItem::Super { x, y, .. } => (*x, *y),
            };
            let vx = nx - cx;
            let vy = ny - cy;
            let aligned = if dx != 0 {
                vx * dx as f64 > 0.01 && vy.abs() <= vx.abs() + 0.15
            } else {
                vy * dy as f64 > 0.01 && vx.abs() <= vy.abs() + 0.15
            };
            if !aligned {
                continue;
            }
            let dist = vx.hypot(vy);
            if best.map(|(_, d)| dist < d).unwrap_or(true) {
                best = Some((vis_i, dist));
            }
        }
        let next_item = if let Some((vis_i, _)) = best {
            items.get(vis_i).cloned()
        } else {
            let n = items.len();
            let next = if dx < 0 || dy < 0 {
                (cur_i + n - 1) % n
            } else if dx > 0 || dy > 0 {
                (cur_i + 1) % n
            } else {
                cur_i
            };
            items.get(next).cloned()
        };
        self.pin_tna_selection(next_item);
    }

    fn on_esc(&mut self) {
        if self.modal {
            self.modal = false;
            return;
        }
        if self.editing {
            self.editing = false;
            return;
        }
        if self.case_page == CasePage::Network {
            if self.tna_find.is_some() {
                self.tna_find = None;
                return;
            }
            if self.chat_report.is_some() {
                self.persist_visible_chat();
                self.chat_report = None;
                self.tna_report = None;
                self.load_transcript("desk");
                self.scroll_back = 0;
                self.focus = Focus::Graph;
                self.ensure_tna_snapshot(false);
                return;
            }
            self.case_page = CasePage::Closed;
            self.fields.clear();
            self.focus = Focus::Prompt;
            return;
        }
        if self.widget().is_some() {
            self.module = Some(ModuleId::Cases);
            self.fields.clear();
            self.editing = false;
            self.focus = Focus::Prompt;
            return;
        }
        if self.chat_report.is_some() || self.chat_case.is_some() {
            self.persist_visible_chat();
            self.chat_report = None;
            self.chat_case = None;
            self.load_transcript("desk");
            self.scroll_back = 0;
            self.focus = Focus::Prompt;
        }
    }

    fn on_modal_key(&mut self, key: KeyEvent) -> bool {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if key.code == KeyCode::Esc || (ctrl && key.code == KeyCode::Char('p')) {
            self.modal = false;
            return false;
        }
        let n = self.filtered_modules().len();
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                if n > 0 {
                    self.modal_sel = (self.modal_sel + n - 1) % n;
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if n > 0 {
                    self.modal_sel = (self.modal_sel + 1) % n;
                }
            }
            KeyCode::Enter => {
                if let Some(module) = self.filtered_modules().get(self.modal_sel).copied() {
                    self.modal = false;
                    self.open_module(module);
                }
            }
            KeyCode::Backspace => {
                self.modal_query.pop();
                self.modal_sel = 0;
            }
            KeyCode::Char(c) if !ctrl => {
                self.modal_query.push(c);
                self.modal_sel = 0;
            }
            _ => {}
        }
        false
    }

    fn on_launcher_key(&mut self, key: KeyEvent) -> bool {
        let n = ModuleId::all().len();
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.launcher_sel = (self.launcher_sel + n - 1) % n,
            KeyCode::Down | KeyCode::Char('j') => self.launcher_sel = (self.launcher_sel + 1) % n,
            KeyCode::Enter => self.open_module(ModuleId::all()[self.launcher_sel]),
            KeyCode::Char(c) if c.is_ascii_digit() && c != '0' => {
                let i = (c as usize) - ('1' as usize);
                if i < n {
                    self.launcher_sel = i;
                    self.open_module(ModuleId::all()[i]);
                }
            }
            _ => {}
        }
        false
    }

    fn on_canvas_key(&mut self, key: KeyEvent) -> bool {
        if self.case_page == CasePage::Network {
            return self.on_tna_key(key);
        }
        if self.case_page == CasePage::Brain && self.form_module() == Some(ModuleId::Brain) {
            return self.on_brain_key(key);
        }
        if !self.fields.is_empty()
            && matches!(
                self.form_module(),
                Some(
                    ModuleId::Providers
                        | ModuleId::Gmail
                        | ModuleId::Settings
                        | ModuleId::Brain
                        | ModuleId::Osint,
                )
            )
        {
            let n = self.fields.len();
            match key.code {
                KeyCode::Up | KeyCode::Char('k') => self.field_sel = (self.field_sel + n - 1) % n,
                KeyCode::Down | KeyCode::Char('j') => self.field_sel = (self.field_sel + 1) % n,
                KeyCode::Enter => self.activate_field(),
                KeyCode::Left | KeyCode::Right
                    if matches!(
                        self.module,
                        Some(ModuleId::Cases | ModuleId::Providers | ModuleId::System)
                    ) =>
                {
                    let delta = if key.code == KeyCode::Left { -1 } else { 1 };
                    self.cycle_group_page(delta);
                }
                KeyCode::Char('x') | KeyCode::Delete
                    if self.form_module() == Some(ModuleId::Osint) =>
                {
                    self.delete_osint_source();
                }
                KeyCode::Char('t') if self.form_module() == Some(ModuleId::Osint) => {
                    self.toggle_osint_source();
                }
                KeyCode::Char('[') | KeyCode::Char(']')
                    if self.form_module() == Some(ModuleId::Osint) =>
                {
                    let n = self.settings.sources.len();
                    if n > 0 {
                        let delta = if key.code == KeyCode::Char('[') {
                            n - 1
                        } else {
                            1
                        };
                        self.source_sel = (self.source_sel + delta) % n;
                    }
                }
                _ => {}
            }
            return false;
        }
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.scroll_chat(1),
            KeyCode::Down | KeyCode::Char('j') => self.scroll_chat(-1),
            KeyCode::Char('r') if self.widget() == Some(ModuleId::Hardware) => {
                self.spawn_hardware(true)
            }
            _ => {}
        }
        if self.on_case_desk() {
            let n = self.report_rows().len();
            if n > 0 {
                match key.code {
                    KeyCode::Char('K') => {
                        self.report_sel = (self.report_sel + n - 1) % n;
                    }
                    KeyCode::Char('J') => {
                        self.report_sel = (self.report_sel + 1) % n;
                    }
                    _ => {}
                }
            }
        }
        if matches!(
            self.module,
            Some(ModuleId::Cases | ModuleId::Providers | ModuleId::System)
        ) {
            match key.code {
                KeyCode::Left => self.cycle_group_page(-1),
                KeyCode::Right => self.cycle_group_page(1),
                _ => {}
            }
        }
        false
    }

    fn on_brain_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Left => self.cycle_group_page(-1),
            KeyCode::Right => self.cycle_group_page(1),
            KeyCode::Up | KeyCode::Char('k') => self.move_brain_sel(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_brain_sel(1),
            KeyCode::Enter => self.open_memory_card(),
            _ => {}
        }
        false
    }

    fn open_memory_card(&mut self) {
        let Some(memory) = self.shown_memories().get(self.brain_sel).cloned() else {
            return;
        };
        self.brain_edit_id = Some(memory.id);
        self.brain_card = true;
        self.editing = false;
    }

    fn on_brain_card_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Esc | KeyCode::Enter => self.close_brain_card(),
            _ => {}
        }
        false
    }

    fn close_brain_card(&mut self) {
        self.brain_card = false;
        self.editing = false;
        self.brain_edit_id = None;
        self.brain_action = 0;
        if self.status.starts_with("saved ") || self.status == "new memory" {
            self.status = "ready".into();
        }
    }

    fn move_brain_sel(&mut self, delta: isize) {
        let n = self.shown_memories().len();
        if n == 0 {
            self.brain_sel = 0;
            return;
        }
        self.brain_sel = (self.brain_sel as isize + delta).rem_euclid(n as isize) as usize;
    }

    fn on_case_desk(&self) -> bool {
        matches!(self.module, None | Some(ModuleId::Cases))
    }

    fn start_case_from_prompt(&mut self) {
        let query = self.prompt.trim().to_string();
        if query.is_empty() {
            self.status = "Type a research query, then press +".into();
            self.focus = Focus::Prompt;
            return;
        }
        self.prompt.clear();
        self.cursor = 0;
        self.open_scope(query, true, true);
    }

    fn open_scope(&mut self, query: String, echo_on_desk: bool, restore_prompt: bool) {
        self.scope = Some(ScopeDraft {
            query,
            echo_on_desk,
            restore_prompt,
            facts: self.settings.facts,
            web: self.settings.web,
            news: self.settings.news,
            domain: self.settings.domain,
            social: self.settings.social,
            identity: self.settings.identity,
            selected: 0,
        });
        self.confirm_query = None;
        self.confirm_sel = 0;
        self.status = "choose sources for this case".into();
    }

    fn on_scope_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                if let Some(scope) = self.scope.as_mut() {
                    scope.selected = (scope.selected + 5) % 6;
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if let Some(scope) = self.scope.as_mut() {
                    scope.selected = (scope.selected + 1) % 6;
                }
            }
            KeyCode::Char(' ') => {
                if let Some(scope) = self.scope.as_mut() {
                    let flag = match scope.selected {
                        0 => &mut scope.facts,
                        1 => &mut scope.web,
                        2 => &mut scope.news,
                        3 => &mut scope.domain,
                        4 => &mut scope.social,
                        _ => &mut scope.identity,
                    };
                    *flag = !*flag;
                }
            }
            KeyCode::Enter => {
                if let Some(scope) = self.scope.take() {
                    let plan = self.plan_for_scope(&scope);
                    self.launch_case_worker(scope.query, scope.echo_on_desk, plan);
                }
            }
            KeyCode::Esc => {
                if let Some(scope) = self.scope.take() {
                    if scope.restore_prompt {
                        self.prompt = scope.query;
                        self.cursor = self.prompt.chars().count();
                        self.focus = Focus::Prompt;
                    }
                    self.status = "case worker cancelled".into();
                }
            }
            _ => {}
        }
        false
    }

    fn plan_for_scope(&self, scope: &ScopeDraft) -> SourcePlan {
        let mut plan = self.settings.source_plan();
        plan.facts = scope.facts;
        plan.web = scope.web;
        plan.news = scope.news;
        plan.domain = scope.domain;
        plan.social = scope.social;
        plan.identity = scope.identity;
        plan
    }

    fn launch_case_worker(&mut self, query: String, echo_on_desk: bool, plan: SourcePlan) {
        let case = match self.store.create_case(&query) {
            Ok(case) => case,
            Err(err) => {
                self.status = err.to_string();
                return;
            }
        };
        let _ = self.reload_lists();
        self.case_sel = self
            .cases
            .iter()
            .position(|item| item.id == case.id)
            .unwrap_or(0);
        self.module = Some(ModuleId::Cases);
        self.case_page = CasePage::Closed;
        self.fields.clear();
        self.chat_case = None;
        self.pending_reports.push(PendingReport {
            case_id: case.id.clone(),
            title: query.clone(),
            failed: None,
        });
        self.report_sel = 0;
        self.load_transcript("desk");
        let ack = "Case worker started to carry out the research.";
        self.append_to_session(&case.id, "user", &query);
        self.append_to_session(&case.id, "assistant", ack);
        if echo_on_desk {
            self.append_to_session("desk", "user", &query);
        }
        self.append_to_session("desk", "assistant", ack);
        self.status = "ready".into();
        self.log_event("task", &format!("case research {}", case.title));
        let case_id = case.id.clone();
        let report_dir = report_dir(&self.settings);
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = match argos_osint_core::search::research(&query, &plan).await {
                Ok(outcome) => {
                    let hits = outcome.hits;
                    let md = report::source_pack(&query, Some(&case_id), &query, &hits);
                    match report::write_report(&report_dir, &query, Some(&case_id), &md) {
                        Ok(meta) => Ok((summarize_hits(&hits), Some(meta))),
                        Err(err) => Ok((
                            format!(
                                "{}

Could not write the report: {err}",
                                summarize_hits(&hits)
                            ),
                            None,
                        )),
                    }
                }
                Err(err) => Err(err),
            };
            let _ = tx.send(AppMsg::Research { case_id, result });
        });
    }

    fn finish_research(
        &mut self,
        case_id: String,
        result: Result<(String, Option<ReportMeta>), String>,
    ) {
        self.status = "ready".into();
        match result {
            Ok((_summary, Some(report))) => {
                self.pending_reports
                    .retain(|pending| pending.case_id != case_id);
                self.log_event("task", &format!("report {}", report.path));
                let _ = self.store.add_report(&report);
                let _ = self.reload_lists();
                self.after_report_filed(&report.id);
            }
            Ok((summary, None)) => self.mark_task_failed(&case_id, &summary),
            Err(err) => self.mark_task_failed(&case_id, &err),
        }
    }

    fn mark_task_failed(&mut self, case_id: &str, reason: &str) {
        self.log_event("task", &format!("case {case_id} failed: {reason}"));
        let reason = reason
            .split('\n')
            .find(|line| !line.trim().is_empty())
            .unwrap_or("research failed")
            .chars()
            .take(160)
            .collect::<String>();
        if let Some(task) = self
            .pending_reports
            .iter_mut()
            .find(|pending| pending.case_id == case_id)
        {
            task.failed = Some(reason);
        }
    }

    fn append_to_session(&mut self, session_id: &str, role: &str, body: &str) {
        let _ = self.store.append_message(session_id, role, body);
        if self.transcripts.contains_key(session_id) {
            if let Some(lines) = self.transcripts.get_mut(session_id) {
                lines.push(ChatLine {
                    role: role.into(),
                    body: body.into(),
                    created_at: String::new(),
                });
            }
        } else {
            self.load_transcript(session_id);
        }
        if self.session_id() == session_id {
            self.scroll_back = 0;
        }
    }

    fn delete_highlighted_case(&mut self) {
        let Some(case_id) = self
            .report_rows()
            .get(self.report_sel)
            .and_then(|row| row.case_id().map(|id| id.to_string()))
        else {
            self.status = "no case to delete".into();
            return;
        };
        let Some(case) = self.cases.iter().find(|case| case.id == case_id).cloned() else {
            self.pending_reports
                .retain(|pending| pending.case_id != case_id);
            self.status = "no case to delete".into();
            return;
        };
        if let Err(err) = self.store.delete_case(&case.id) {
            self.status = err.to_string();
            return;
        }
        if self.chat_case.as_deref() == Some(case.id.as_str()) {
            self.chat_case = None;
        }
        self.pending_reports
            .retain(|pending| pending.case_id != case.id);
        let _ = self.reload_lists();
        self.load_transcript(&self.session_id());
        self.status = format!("deleted {}", case.title);
        self.log_event("task", &format!("deleted case {}", case.title));
    }

    fn save_osint_toggles(&mut self) {
        self.settings.facts = self.field_value("facts").eq_ignore_ascii_case("yes");
        self.settings.web = self.field_value("web").eq_ignore_ascii_case("yes");
        self.settings.news = self.field_value("news").eq_ignore_ascii_case("yes");
        self.settings.domain = self.field_value("domain").eq_ignore_ascii_case("yes");
        self.settings.social = self.field_value("social").eq_ignore_ascii_case("yes");
        self.settings.identity = self.field_value("identity").eq_ignore_ascii_case("yes");
        self.settings.searx_url = self.field_value("searx_url");
        self.settings.brave_key = self.field_value("brave_key");
        self.settings.tavily_key = self.field_value("tavily_key");
        self.settings.youtube_key = self.field_value("youtube_key");
        self.settings.github_token = self.field_value("github_token");
        match self.settings.save() {
            Ok(()) => self.status = "saved OSINT sources".into(),
            Err(err) => {
                self.status = "could not save OSINT sources".into();
                self.log_event("system", &format!("osint sources: {err}"));
            }
        }
    }

    fn add_osint_source(&mut self) {
        let name = self.field_value("source_name").trim().to_string();
        let url_template = self.field_value("source_url").trim().to_string();
        if name.is_empty() || !url_template.contains("{query}") {
            self.status = "name and a URL template containing {query} are required".into();
            return;
        }
        let sample = url_template.replace("{query}", "example");
        if let Err(err) = argos_osint_core::search::check_public_http(&sample) {
            self.status = err;
            return;
        }
        self.settings
            .sources
            .push(argos_osint_core::search::OsintSource {
                name,
                url_template,
                enabled: true,
            });
        if let Err(err) = self.settings.save() {
            self.status = err.to_string();
            return;
        }
        self.field_set("source_name", String::new());
        self.field_set("source_url", String::new());
        self.source_sel = self.settings.sources.len().saturating_sub(1);
        self.status = "added OSINT source".into();
    }

    fn delete_osint_source(&mut self) {
        if self.settings.sources.is_empty() {
            return;
        }
        let index = self.source_sel.min(self.settings.sources.len() - 1);
        self.settings.sources.remove(index);
        if self.source_sel >= self.settings.sources.len() {
            self.source_sel = self.settings.sources.len().saturating_sub(1);
        }
        let _ = self.settings.save();
    }

    fn toggle_osint_source(&mut self) {
        if let Some(source) = self.settings.sources.get_mut(self.source_sel) {
            source.enabled = !source.enabled;
        }
        let _ = self.settings.save();
    }

    fn bind_case(&mut self) {
        if let Some(case) = self.cases.get(self.case_sel) {
            let id = case.id.clone();
            self.chat_case = Some(id.clone());
            self.load_transcript(&id);
        }
        self.scroll_back = 0;
    }

    fn on_prompt_key(&mut self, key: KeyEvent) -> bool {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let menu = self.prompt.starts_with('/') && !self.prompt.contains(' ');
        match key.code {
            KeyCode::Enter => self.submit(),
            KeyCode::Backspace => {
                if self.cursor > 0 {
                    let mut chars: Vec<char> = self.prompt.chars().collect();
                    chars.remove(self.cursor - 1);
                    self.cursor -= 1;
                    self.prompt = chars.into_iter().collect();
                }
            }
            KeyCode::Left => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Right => self.cursor = (self.cursor + 1).min(self.prompt.chars().count()),
            KeyCode::Up if menu => self.cycle_slash(-1),
            KeyCode::Down if menu => self.cycle_slash(1),
            KeyCode::Up => self.hist(-1),
            KeyCode::Down => self.hist(1),
            KeyCode::Char('u') if ctrl => {
                self.prompt.clear();
                self.cursor = 0;
            }
            KeyCode::Char(c) if !ctrl => {
                let mut chars: Vec<char> = self.prompt.chars().collect();
                let at = self.cursor.min(chars.len());
                chars.insert(at, c);
                self.cursor = at + 1;
                self.prompt = chars.into_iter().collect();
                self.hist_pos = None;
            }
            _ => {}
        }
        false
    }

    fn cycle_slash(&mut self, delta: isize) {
        let mut card = session::slash_menu(&self.prompt);
        card.selected = card
            .options
            .iter()
            .position(|o| self.prompt.trim_start_matches('/') == o.id)
            .unwrap_or(0);
        card.move_sel(delta);
        if let Some(opt) = card.selected() {
            self.prompt = opt.label.clone();
            self.cursor = self.prompt.chars().count();
        }
    }

    fn hist(&mut self, delta: isize) {
        if self.history.is_empty() {
            return;
        }
        let len = self.history.len() as isize;
        let cur = self.hist_pos.map(|p| p as isize).unwrap_or(len);
        let next = (cur + delta).clamp(0, len);
        if next == len {
            self.hist_pos = None;
            return;
        }
        self.hist_pos = Some(next as usize);
        self.prompt = self.history[next as usize].clone();
        self.cursor = self.prompt.chars().count();
    }

    fn submit(&mut self) {
        let line = self.prompt.trim().to_string();
        if line.is_empty() {
            return;
        }
        self.history.push(line.clone());
        self.hist_pos = None;
        self.prompt.clear();
        self.cursor = 0;
        if line.starts_with('/') {
            self.run_slash(&line);
        } else if self.routes_desk_message() {
            self.route_desk_message(line);
        } else {
            self.spawn_turn(line);
        }
    }

    fn routes_desk_message(&self) -> bool {
        self.on_case_desk() && self.chat_case.is_none() && self.chat_report.is_none()
    }

    fn open_report_chat(&mut self, id: &str) {
        let Some(report) = self.reports.iter().find(|report| report.id == id).cloned() else {
            self.status = "that report is no longer on file".into();
            return;
        };
        let session = format!("report:{}", report.id);
        if self
            .store
            .ensure_session(&session, &report.title, "report")
            .is_err()
        {
            self.status = "could not open the report chat".into();
            return;
        }
        if self.chat_report.as_deref() != Some(report.id.as_str()) {
            self.persist_visible_chat();
        }
        self.chat_case = None;
        self.chat_report = Some(report.id.clone());
        if self.case_page == CasePage::Network {
            self.module = Some(ModuleId::Cases);
            self.scroll_back = 0;
            self.focus = Focus::Graph;
            self.tna_report = None;
            self.ensure_tna_snapshot(true);
        } else {
            self.case_page = CasePage::Closed;
            self.module = Some(ModuleId::Cases);
            self.scroll_back = 0;
            self.focus = Focus::Prompt;
        }
        self.transcripts.remove(&session);
        self.load_transcript(&self.session_id());
        self.status = "ready".into();
    }

    fn route_desk_message(&mut self, text: String) {
        if self.running {
            self.status = "busy".into();
            return;
        }
        if prompt::classify(&text) == Intent::Remember {
            self.spawn_turn(text);
            return;
        }
        let facts = brain::recall_report_facts(&self.memories, &text, 4);
        if !facts.is_empty() {
            let material = memory_answer_material(self, &facts);
            if brain::insists_on_new_case(&text) {
                self.push_line("user", &text);
                self.desk_memory_answer = Some(material);
                self.confirm_query = Some(text);
                self.confirm_sel = 0;
            } else {
                self.desk_memory_answer = None;
                self.spawn_answered_turn(text, material, true, true, true);
            }
            return;
        }
        if let Some(pending) = self
            .pending_reports
            .iter()
            .filter(|pending| pending.failed.is_none())
            .find(|pending| report::title_matches(&text, &pending.title))
        {
            let title = pending.title.clone();
            self.push_line("user", &text);
            self.push_line(
                "assistant",
                &format!(
                    "A case worker is already researching “{title}”. The report list shows it as pending."
                ),
            );
            return;
        }
        self.push_line("user", &text);
        self.desk_memory_answer = None;
        self.confirm_query = Some(text);
        self.confirm_sel = 0;
    }

    fn on_confirm_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.confirm_sel = 0,
            KeyCode::Down | KeyCode::Char('j') => self.confirm_sel = 1,
            KeyCode::Char('y') => self.confirm_case(true),
            KeyCode::Char('n') | KeyCode::Esc => self.confirm_case(false),
            KeyCode::Enter => self.confirm_case(self.confirm_sel == 0),
            _ => {}
        }
        false
    }

    fn confirm_case(&mut self, start: bool) {
        let Some(query) = self.confirm_query.take() else {
            return;
        };
        if start {
            self.desk_memory_answer = None;
            self.open_scope(query, false, false);
        } else if let Some(material) = self.desk_memory_answer.take() {
            self.spawn_answered_turn(query, material, false, true, true);
        } else {
            self.spawn_answered_turn(query, String::new(), false, false, false);
        }
    }

    fn run_slash(&mut self, line: &str) {
        let rest = line.trim_start_matches('/').trim();
        let mut parts = rest.splitn(2, char::is_whitespace);
        let cmd = parts.next().unwrap_or("").to_lowercase();
        let arg = parts.next().unwrap_or("").trim().to_string();
        match cmd.as_str() {
            "help" | "?" => self.help = true,
            "quit" | "exit" | "q" => self.quit = true,
            "dashboard" | "home" => self.on_esc(),
            "new" => {
                if arg.is_empty() {
                    self.status = "Type a research query, then press +".into();
                } else {
                    self.open_scope(arg, true, true);
                }
            }
            "use" => match session::resolve_case(&self.cases, &arg) {
                Some(case) => {
                    self.case_sel = self.cases.iter().position(|c| c.id == case.id).unwrap_or(0);
                    self.open_module(ModuleId::Cases);
                    self.bind_case();
                }
                None => self.push_line("assistant", "No single case matches that query."),
            },
            "search" => {
                if arg.is_empty() {
                    self.push_line("assistant", "Usage: /search <query>");
                } else {
                    self.spawn_search(arg);
                }
            }
            "report" => self.write_visible_report(&arg),
            "hardware" => {
                self.open_module(ModuleId::Hardware);
                if arg == "fresh" {
                    self.spawn_hardware(true);
                }
            }
            "log" => self.open_module(ModuleId::Log),
            "settings" => self.open_module(ModuleId::Settings),
            "system" => self.open_module(ModuleId::System),
            "provider" | "login" => self.open_module(ModuleId::Providers),
            "osint" | "sources" => self.open_module(ModuleId::Osint),
            "model" | "m" | "models" => {
                if arg.is_empty() || cmd == "models" {
                    self.open_model_picker();
                } else {
                    match provider::resolve_model_choice(&self.model_choices(), &arg) {
                        Some(id) => self.select_model(&id),
                        None => self.push_line(
                            "assistant",
                            &format!("No single model matches {arg}. /model opens the picker."),
                        ),
                    }
                }
            }
            "network" => {
                self.open_module(ModuleId::Cases);
                self.select_case_page(CasePage::Network);
            }
            "find" => {
                if self.case_page != CasePage::Network {
                    self.select_case_page(CasePage::Network);
                }
                self.tna_find = Some(String::new());
                self.focus = Focus::Graph;
            }
            "brain" => {
                if arg.is_empty() {
                    self.open_module(ModuleId::Brain);
                } else {
                    let (category, text) = brain::parse_typed_memory(&arg);
                    if text.is_empty() {
                        self.open_module(ModuleId::Brain);
                    } else {
                        match self.store.add_memory_typed(&text, category, false) {
                            Ok(mem) => {
                                let _ = self.reload_lists();
                                self.push_line(
                                    "assistant",
                                    &format!("Remembered [{}]: {}", mem.category, mem.text),
                                );
                            }
                            Err(err) => {
                                self.log_event("system", &format!("brain save failed: {err}"));
                                self.push_line(
                                    "assistant",
                                    "Could not store that memory. The detail is in the System log.",
                                );
                            }
                        }
                    }
                }
            }
            "gmail" => self.open_module(ModuleId::Gmail),
            "voice" => {
                self.settings.modality = "voice".into();
                let _ = self.settings.save();
                self.push_line(
                    "assistant",
                    "Modality is voice. Ctrl+R records, Enter sends the transcript.",
                );
            }
            "text" => {
                self.settings.modality = "text".into();
                let _ = self.settings.save();
                self.push_line("assistant", "Modality is text.");
            }
            "open" => {
                if let Some(module) = ModuleId::from_name(&arg) {
                    self.open_module(module);
                } else {
                    self.push_line("assistant", "Unknown app. Ctrl+P lists them.");
                }
            }
            "clear" => self.clear_chat_view(),
            other => self.push_line(
                "assistant",
                &format!("Unknown command /{other}. /help lists them."),
            ),
        }
    }

    fn clear_chat_view(&mut self) {
        if self.running {
            self.status = "busy".into();
            return;
        }
        let id = self.session_id();
        let _ = self.store.clear_messages(&id);
        self.transcripts.insert(id, Vec::new());
        self.scroll_back = 0;
        self.status = match self.chat_report {
            Some(_) => "report chat cleared".into(),
            None if self.chat_case.is_none() => "case desk chat cleared".into(),
            None => "chat cleared".into(),
        };
    }

    fn write_visible_report(&mut self, title: &str) {
        let title = if title.is_empty() {
            self.view_name()
        } else {
            title.to_string()
        };
        let mut body = String::new();
        for line in self.transcript() {
            body.push_str(&format!("**{}:** {}\n\n", line.role, line.body));
        }
        let md = report::render_report(&title, self.case_id().as_deref(), "", &body, &[]);
        match report::write_report(
            &report_dir(&self.settings),
            &title,
            self.case_id().as_deref(),
            &md,
        ) {
            Ok(meta) => {
                let _ = self.store.add_report(&meta);
                let _ = self.reload_lists();
                self.after_report_filed(&meta.id);
                self.push_line("assistant", &format!("Report: {}", meta.path));
            }
            Err(err) => {
                self.log_event("task", &format!("report write failed: {err}"));
                self.push_line(
                    "assistant",
                    "The report could not be written. The detail is in the System log.",
                );
            }
        }
    }

    fn spawn_search(&mut self, query: String) {
        if self.running {
            self.status = "busy".into();
            return;
        }
        self.running = true;
        self.status = "searching · Ctrl+C cancels".into();
        self.push_line("user", &format!("/search {query}"));
        self.push_line("assistant", "");
        self.log_event("search", &format!("search {query}"));
        let plan = self.settings.source_plan();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = argos_osint_core::search::web_search(&query, &plan).await;
            let _ = tx.send(AppMsg::Search { query, result });
        });
    }

    fn spawn_turn(&mut self, text: String) {
        self.spawn_answered_turn(text, String::new(), true, false, false);
    }

    fn spawn_answered_turn(
        &mut self,
        text: String,
        prior_reports: String,
        echo_user: bool,
        evidence_only: bool,
        from_memory: bool,
    ) {
        if self.running {
            self.status = "busy".into();
            return;
        }
        if prior_reports.is_empty() && prompt::classify(&text) == Intent::Remember {
            let fact = prompt::remember_text(&text);
            self.push_line("user", &text);
            if let Ok(mem) = self.store.add_memory(&fact) {
                self.memories.insert(0, mem);
                self.push_line("assistant", &format!("Remembered: {fact}"));
            }
            return;
        }
        self.running = true;
        self.cancel = Arc::new(AtomicBool::new(false));
        self.status = "starting".into();
        if echo_user {
            self.push_line("user", &text);
        }
        self.push_line("assistant", "");
        self.log_event("api", &format!("chat {}", self.active_model()));
        let (prior_reports, evidence_only, from_memory, memories) =
            if let Some(report) = self.open_report().cloned() {
                (
                    prior_report_material(std::slice::from_ref(&report)),
                    true,
                    false,
                    Vec::new(),
                )
            } else {
                (
                    prior_reports,
                    evidence_only,
                    from_memory,
                    self.memories.clone(),
                )
            };
        let id = self.session_id();
        let history = self
            .transcript()
            .iter()
            .rev()
            .skip(2)
            .take(16)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .filter(|l| l.role == "user" || l.role == "assistant")
            .map(|l| HistMsg {
                role: l.role.clone(),
                content: l.body.clone(),
            })
            .collect();
        let input = TurnInput {
            session_id: id,
            user_text: text,
            history,
            memories,
            view_name: self.view_name(),
            view_context: self.view_context(),
            hardware_line: self.hardware.one_line(),
            modality: self.settings.modality.clone(),
            provider: Some(self.role_secret(&self.settings.writer_model)),
            tool_provider: Some(self.role_secret(&self.settings.tool_model)),
            plan: self.settings.source_plan(),
            report_dir: report_dir(&self.settings),
            case_id: self.case_id(),
            gmail: self.auth.gmail.as_ref().map(GmailConfig::from),
            prior_reports,
            evidence_only,
            from_memory,
        };
        let tx = self.tx.clone();
        let cancel = Arc::clone(&self.cancel);
        tokio::spawn(async move {
            let (atx, mut arx) = unbounded_channel();
            let worker = tokio::spawn(async move {
                agent::run_turn(input, atx, cancel).await;
            });
            while let Some(ev) = arx.recv().await {
                if tx.send(AppMsg::Turn(ev)).is_err() {
                    break;
                }
            }
            let _ = worker.await;
        });
    }

    pub fn open_module(&mut self, module: ModuleId) {
        match module {
            ModuleId::Brain => {
                self.module = Some(ModuleId::Cases);
                self.case_page = CasePage::Brain;
            }
            ModuleId::Log => {
                self.module = Some(ModuleId::System);
                self.system_page = SystemPage::Log;
            }
            ModuleId::Hardware => {
                self.module = Some(ModuleId::System);
                self.system_page = SystemPage::Hardware;
            }
            ModuleId::Settings => {
                self.module = Some(ModuleId::System);
                self.system_page = SystemPage::Settings;
            }
            ModuleId::System => {
                self.module = Some(ModuleId::System);
                self.system_page = SystemPage::Log;
            }
            ModuleId::Reports => {
                self.module = Some(ModuleId::Cases);
                self.case_page = CasePage::Closed;
            }
            ModuleId::Cases => {
                self.module = Some(ModuleId::Cases);
                self.case_page = CasePage::Closed;
            }
            ModuleId::Gmail => {
                self.module = Some(ModuleId::Providers);
                self.provider_page = ProviderPage::Mail;
            }
            ModuleId::Osint => {
                self.module = Some(ModuleId::Providers);
                self.provider_page = ProviderPage::Osint;
            }
            ModuleId::Providers => {
                self.module = Some(ModuleId::Providers);
                self.provider_page = ProviderPage::Llm;
            }
        }
        let module = self.module.unwrap_or(ModuleId::Cases);
        self.scroll_back = 0;
        self.editing = false;
        if module == ModuleId::Cases && self.case_page == CasePage::Closed {
            self.fields.clear();
            self.focus = Focus::Prompt;
            self.load_transcript(&self.session_id());
            return;
        }
        self.focus = Focus::Canvas;
        self.load_group_fields();
        if module == ModuleId::System && self.system_page == SystemPage::Hardware {
            self.spawn_hardware(false);
        }
    }

    fn load_group_fields(&mut self) {
        match self.form_module() {
            Some(module) => self.load_fields(module),
            None => self.fields.clear(),
        }
    }

    fn cycle_group_page(&mut self, delta: isize) {
        match self.module {
            Some(ModuleId::Cases) => {
                let pages = CasePage::all();
                let index = pages
                    .iter()
                    .position(|page| *page == self.case_page)
                    .unwrap_or(0);
                let next = (index as isize + delta).rem_euclid(pages.len() as isize) as usize;
                self.case_page = pages[next];
            }
            Some(ModuleId::Providers) => {
                let pages = ProviderPage::all();
                let index = pages
                    .iter()
                    .position(|page| *page == self.provider_page)
                    .unwrap_or(0);
                let next = (index as isize + delta).rem_euclid(pages.len() as isize) as usize;
                self.provider_page = pages[next];
            }
            Some(ModuleId::System) => {
                let pages = SystemPage::all();
                let index = pages
                    .iter()
                    .position(|page| *page == self.system_page)
                    .unwrap_or(0);
                let next = (index as isize + delta).rem_euclid(pages.len() as isize) as usize;
                self.system_page = pages[next];
                if self.system_page == SystemPage::Hardware {
                    self.spawn_hardware(false);
                }
            }
            _ => return,
        }
        self.field_sel = 0;
        self.editing = false;
        self.focus = Focus::Canvas;
        self.load_group_fields();
    }

    fn load_fields(&mut self, module: ModuleId) {
        self.fields.clear();
        self.field_sel = 0;
        match module {
            ModuleId::Osint => {
                self.fields = vec![
                    field(
                        "facts",
                        "Facts (enter toggles)",
                        yes_no(self.settings.facts),
                        false,
                    ),
                    field(
                        "web",
                        "Web (enter toggles)",
                        yes_no(self.settings.web),
                        false,
                    ),
                    field(
                        "news",
                        "News (enter toggles)",
                        yes_no(self.settings.news),
                        false,
                    ),
                    field(
                        "domain",
                        "Domain (enter toggles)",
                        yes_no(self.settings.domain),
                        false,
                    ),
                    field(
                        "social",
                        "Social (enter toggles)",
                        yes_no(self.settings.social),
                        false,
                    ),
                    field(
                        "identity",
                        "Identity (enter toggles)",
                        yes_no(self.settings.identity),
                        false,
                    ),
                    field(
                        "searx_url",
                        "SearXNG URL (empty uses DuckDuckGo)",
                        self.settings.searx_url.clone(),
                        false,
                    ),
                    field(
                        "brave_key",
                        "Brave API key",
                        self.settings.brave_key.clone(),
                        true,
                    ),
                    field(
                        "tavily_key",
                        "Tavily API key",
                        self.settings.tavily_key.clone(),
                        true,
                    ),
                    field(
                        "youtube_key",
                        "YouTube API key",
                        self.settings.youtube_key.clone(),
                        true,
                    ),
                    field(
                        "github_token",
                        "GitHub token",
                        self.settings.github_token.clone(),
                        true,
                    ),
                    field("source_name", "Extra source name", String::new(), false),
                    field(
                        "source_url",
                        "Extra source URL template with {query}",
                        String::new(),
                        false,
                    ),
                    field("__add", "Add extra source", "enter".into(), false),
                    field("__save", "Save source toggles", "enter".into(), false),
                ];
            }
            ModuleId::Brain => {
                self.fields = vec![
                    field("category", "Type  1-7", "fact".into(), false),
                    field("text", "Memory", String::new(), false),
                    field("pin", "Pin for recall", "no".into(), false),
                    field("__save", "Add this memory", "s".into(), false),
                ];
                self.brain_edit_id = None;
                self.set_brain_action_label();
            }
            ModuleId::Providers => {
                let secret = self.slot_secret();
                let fallback = provider::preset("local").expect("local preset");
                let kind = secret
                    .as_ref()
                    .map(|slot| provider::normalize_kind(&slot.kind))
                    .filter(|kind| provider::preset(kind).is_some())
                    .unwrap_or_else(|| "local".into());
                let chosen = provider::preset(&kind).unwrap_or(fallback);
                self.fields = vec![
                    field("__h_connection", "Connection", String::new(), false),
                    field(
                        "kind",
                        "Provider (enter cycles grok / openai / openrouter / local)",
                        kind,
                        false,
                    ),
                    field(
                        "base_url",
                        "Base URL",
                        secret
                            .as_ref()
                            .map(|slot| slot.base_url.clone())
                            .filter(|url| !url.is_empty())
                            .unwrap_or_else(|| chosen.base_url.into()),
                        false,
                    ),
                    field(
                        "model",
                        "Model",
                        secret
                            .as_ref()
                            .map(|slot| slot.model.clone())
                            .filter(|model| !model.is_empty())
                            .unwrap_or_else(|| {
                                if self.provider_slot == "voice" {
                                    chosen.voice_model.into()
                                } else {
                                    chosen.text_model.into()
                                }
                            }),
                        false,
                    ),
                    field(
                        "api_key",
                        "API key (empty uses XAI_API_KEY, OPENAI_API_KEY, or OPENROUTER_API_KEY)",
                        secret
                            .as_ref()
                            .and_then(|slot| slot.api_key.clone())
                            .unwrap_or_default(),
                        true,
                    ),
                    field(
                        "stt_model",
                        "Voice model",
                        secret
                            .as_ref()
                            .and_then(|slot| slot.stt_model.clone())
                            .unwrap_or_else(|| chosen.voice_model.into()),
                        false,
                    ),
                    field(
                        "__slot",
                        "Slot action: press enter to flip text/voice",
                        self.provider_slot.into(),
                        false,
                    ),
                    field("__save", "Save connection", "enter".into(), false),
                    field("__test", "Test /models", "enter".into(), false),
                    field("__h_roles", "Roles", String::new(), false),
                    field(
                        "__role_writer",
                        "Writer model",
                        self.role_label(&self.settings.writer_model),
                        false,
                    ),
                    field(
                        "__role_tool",
                        "Tool model",
                        self.role_label(&self.settings.tool_model),
                        false,
                    ),
                ];
            }
            ModuleId::Gmail => {
                let g = self.auth.gmail.clone();
                self.fields = vec![
                    field(
                        "email",
                        "Gmail address",
                        g.as_ref().map(|g| g.email.clone()).unwrap_or_default(),
                        false,
                    ),
                    field(
                        "app_password",
                        "App password",
                        g.as_ref()
                            .map(|g| g.app_password.clone())
                            .unwrap_or_default(),
                        true,
                    ),
                    field("__save", "Save Gmail", "enter".into(), false),
                    field("__test", "Test INBOX", "enter".into(), false),
                    field("__mcp", "Write MCP config", "enter".into(), false),
                ];
            }
            ModuleId::Settings => {
                self.fields = vec![
                    field(
                        "searx_url",
                        "SearXNG base URL (empty uses DuckDuckGo)",
                        self.settings.searx_url.clone(),
                        false,
                    ),
                    field(
                        "report_dir",
                        "Report directory (empty uses ./reports)",
                        self.settings.report_dir.clone(),
                        false,
                    ),
                    field("__save", "Save settings", "enter".into(), false),
                ];
            }
            _ => {}
        }
    }

    fn slot_secret(&self) -> Option<ProviderSecret> {
        if self.provider_slot == "voice" {
            self.auth.voice.clone()
        } else {
            self.auth.text.clone()
        }
    }

    fn field_value(&self, key: &str) -> String {
        self.fields
            .iter()
            .find(|f| f.key == key)
            .map(|f| f.value.clone())
            .unwrap_or_default()
    }

    fn activate_field(&mut self) {
        let key = self
            .fields
            .get(self.field_sel)
            .map(|f| f.key.clone())
            .unwrap_or_default();
        if key == "kind" {
            self.cycle_provider();
        } else if key == "__role_writer" {
            self.model_target = ModelTarget::Writer;
            self.open_model_card();
        } else if key == "__role_tool" {
            self.model_target = ModelTarget::Tool;
            self.open_model_card();
        } else if key.starts_with("__h_") {
        } else if key == "model" && provider::is_free_router(&self.field_value("model")) {
            self.model_target = ModelTarget::Connection;
            self.open_free_picker();
        } else if self.form_module() == Some(ModuleId::Brain)
            && matches!(key.as_str(), "category" | "pin")
        {
            self.cycle_brain_field(&key);
        } else if self.form_module() == Some(ModuleId::Osint)
            && matches!(
                key.as_str(),
                "facts" | "web" | "news" | "domain" | "social" | "identity"
            )
        {
            let next = if self.field_value(&key).eq_ignore_ascii_case("yes") {
                "no"
            } else {
                "yes"
            };
            self.field_set(&key, next.into());
        } else if key.starts_with("__") {
            self.run_field_action(&key);
        } else {
            self.editing = true;
        }
    }

    fn cycle_provider(&mut self) {
        let current = provider::normalize_kind(&self.field_value("kind"));
        let order = provider::presets();
        let index = order
            .iter()
            .position(|preset| preset.id == current)
            .unwrap_or(order.len() - 1);
        let next = &order[(index + 1) % order.len()];
        let url = self.field_value("base_url");
        let model = self.field_value("model");
        let stt = self.field_value("stt_model");
        let url_is_default = url.is_empty() || order.iter().any(|preset| preset.base_url == url);
        let model_is_default = model.is_empty()
            || order
                .iter()
                .any(|preset| preset.text_model == model || preset.voice_model == model);
        let stt_is_default = stt.is_empty() || order.iter().any(|preset| preset.voice_model == stt);
        self.field_set("kind", next.id.into());
        if url_is_default {
            self.field_set("base_url", next.base_url.into());
        }
        if model_is_default {
            let model = if self.provider_slot == "voice" {
                next.voice_model
            } else {
                next.text_model
            };
            self.field_set("model", model.into());
        }
        if stt_is_default {
            self.field_set("stt_model", next.voice_model.into());
        }
    }

    fn cycle_brain_field(&mut self, key: &str) {
        match key {
            "category" => {
                let current = brain::normalize_category(&self.field_value("category"));
                let index = brain::CATEGORIES
                    .iter()
                    .position(|name| *name == current)
                    .unwrap_or(0);
                let next = brain::CATEGORIES[(index + 1) % brain::CATEGORIES.len()];
                self.field_set("category", next.into());
            }
            "pin" => {
                let next = if self.field_value("pin").eq_ignore_ascii_case("yes") {
                    "no"
                } else {
                    "yes"
                };
                self.field_set("pin", next.into());
            }
            _ => {}
        }
    }

    fn save_brain_memory(&mut self) {
        let text = self.field_value("text");
        if text.trim().is_empty() {
            self.status = "memory needs text".into();
            return;
        }
        let category = brain::normalize_category(&self.field_value("category"));
        let pinned = self.field_value("pin").eq_ignore_ascii_case("yes");
        let saved = if let Some(id) = self.brain_edit_id.clone() {
            match self.store.update_memory(&id, &text, category, pinned) {
                Ok(true) => Ok(id),
                Ok(false) => {
                    self.status = "that memory is gone".into();
                    return;
                }
                Err(err) => Err(err),
            }
        } else {
            self.store
                .add_memory_typed(&text, category, pinned)
                .map(|memory| memory.id)
        };
        match saved {
            Ok(id) => {
                let _ = self.reload_lists();
                self.brain_edit_id = Some(id.clone());
                self.select_shown_memory(&id);
                self.set_brain_action_label();
                self.status = format!("saved {category} memory");
                self.log_event("system", &format!("brain {category} {text}"));
            }
            Err(err) => self.status = err.to_string(),
        }
    }

    fn select_shown_memory(&mut self, id: &str) {
        if let Some(index) = self
            .shown_memories()
            .iter()
            .position(|memory| memory.id == id)
        {
            self.brain_sel = index;
        }
    }

    fn set_brain_action_label(&mut self) {
        let label = if self.brain_edit_id.is_some() {
            "Update this memory"
        } else {
            "Add this memory"
        };
        if let Some(field) = self.fields.iter_mut().find(|field| field.key == "__save") {
            field.label = label.into();
            field.value = "s".into();
        }
    }

    pub fn shown_memories(&self) -> Vec<Memory> {
        let show = self.brain_filter.as_str();
        self.memories
            .iter()
            .filter(|memory| show.is_empty() || show == "all" || memory.category == show)
            .cloned()
            .collect()
    }

    fn field_set(&mut self, key: &str, value: String) {
        if let Some(field) = self.fields.iter_mut().find(|field| field.key == key) {
            field.value = value;
        }
    }

    fn run_field_action(&mut self, key: &str) {
        if self.form_module() == Some(ModuleId::Brain) && key == "__save" {
            self.save_brain_memory();
            return;
        }
        match (self.module, key) {
            (Some(ModuleId::Providers), "__slot") => {
                self.provider_slot = if self.provider_slot == "text" {
                    "voice"
                } else {
                    "text"
                };
                self.load_fields(ModuleId::Providers);
            }
            (Some(ModuleId::Providers), "__save") => self.save_provider_fields(),
            (Some(ModuleId::Providers), "__test") => self.test_provider(),
            (Some(ModuleId::Gmail), "__save") => self.save_gmail_fields(),
            (Some(ModuleId::Gmail), "__test") => self.test_gmail(),
            (Some(ModuleId::Gmail), "__mcp") => self.write_mcp(),
            (Some(ModuleId::Settings), "__save") | (Some(ModuleId::System), "__save")
                if self.form_module() == Some(ModuleId::Settings) =>
            {
                self.save_settings_fields()
            }
            (Some(ModuleId::Brain), "__save") => self.save_brain_memory(),
            (Some(ModuleId::Osint), "__save") => self.save_osint_toggles(),
            (Some(ModuleId::Osint), "__add") => self.add_osint_source(),
            _ => {}
        }
    }

    fn on_field_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Esc | KeyCode::Enter => self.editing = false,
            KeyCode::Backspace => {
                if let Some(field) = self.fields.get_mut(self.field_sel) {
                    field.value.pop();
                }
            }
            KeyCode::Char(c) => {
                if let Some(field) = self.fields.get_mut(self.field_sel) {
                    field.value.push(c);
                }
            }
            _ => {}
        }
        false
    }

    fn save_provider_fields(&mut self) {
        let kind = provider::normalize_kind(&empty_fallback(&self.field_value("kind"), "local"));
        let chosen = provider::preset(&kind);
        let mut base_url = self.field_value("base_url");
        let mut model = self.field_value("model");
        let picking_free = self.provider_slot == "text" && provider::is_free_router(&model);
        if picking_free {
            model = if self.settings.model.trim().is_empty() {
                chosen
                    .as_ref()
                    .map(|preset| preset.text_model.to_string())
                    .unwrap_or_default()
            } else {
                self.settings.model.clone()
            };
        }
        if let Some(chosen) = chosen {
            if base_url.trim().is_empty() {
                base_url = chosen.base_url.into();
            }
            if model.trim().is_empty() {
                model = if self.provider_slot == "voice" {
                    chosen.voice_model.into()
                } else {
                    chosen.text_model.into()
                };
            }
        }
        let secret = ProviderSecret {
            kind: if chosen.is_some() {
                kind.clone()
            } else {
                "local".into()
            },
            base_url,
            model,
            api_key: Some(self.field_value("api_key")).filter(|key| !key.is_empty()),
            stt_model: Some(self.field_value("stt_model")).filter(|model| !model.is_empty()),
            device: None,
        };
        let missing_key = chosen
            .filter(|preset| preset.key_required)
            .filter(|_| secret.api_key.is_none())
            .filter(|preset| provider::resolved_key(&secret).is_none() || preset.env_key.is_none());
        if self.provider_slot == "text" {
            self.settings.model = secret.model.clone();
            let _ = self.settings.save();
        }
        if self.provider_slot == "voice" {
            self.auth.voice = Some(secret);
        } else {
            self.auth.text = Some(secret);
        }
        match self.auth.save() {
            Ok(()) => {
                let mut note = format!("Saved the {} slot on {kind}.", self.provider_slot);
                if let Some(preset) = missing_key {
                    if let Some(name) = preset.env_key {
                        note.push_str(&format!(" No key stored and {name} is unset."));
                    }
                }
                self.status = format!("saved {} slot", self.provider_slot);
                self.log_event("system", &note);
                if picking_free {
                    self.model_target = ModelTarget::Connection;
                    self.open_free_picker();
                    self.status = "pick a free model".into();
                }
            }
            Err(err) => {
                self.status = "provider save failed".into();
                self.log_event("system", &format!("provider save failed: {err}"));
            }
        }
    }

    fn test_provider(&mut self) {
        self.save_provider_fields();
        let Some(secret) = self.slot_secret() else {
            return;
        };
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = provider::list_models(&secret)
                .await
                .map_err(|e| e.to_string());
            let _ = tx.send(AppMsg::Models(result));
        });
        self.status = "contacting provider".into();
    }

    fn save_gmail_fields(&mut self) {
        let secret = GmailSecret {
            email: self.field_value("email").trim().to_string(),
            app_password: self.field_value("app_password"),
        };
        let cfg = GmailConfig::from(&secret);
        if let Err(err) = gmail::validate(&cfg) {
            self.status = "gmail settings need a correction".into();
            self.log_event("system", &format!("gmail: {err}"));
            return;
        }
        self.auth.gmail = Some(secret);
        match self.auth.save() {
            Ok(()) => {
                self.status = "gmail saved".into();
                self.log_event(
                    "system",
                    "Saved Gmail. The app password stays in ~/.argos/auth.json.",
                );
            }
            Err(err) => {
                self.status = "gmail save failed".into();
                self.log_event("system", &format!("gmail save failed: {err}"));
            }
        }
    }

    fn test_gmail(&mut self) {
        self.save_gmail_fields();
        let Some(cfg) = self.auth.gmail.as_ref().map(GmailConfig::from) else {
            return;
        };
        let tx = self.tx.clone();
        tokio::task::spawn_blocking(move || {
            let result =
                gmail::inbox_count(&cfg).map(|n| format!("INBOX is reachable. {n} messages."));
            let _ = tx.send(AppMsg::GmailTest(result));
        });
        self.status = "checking gmail".into();
    }

    fn write_mcp(&mut self) {
        let exe = std::env::current_exe()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| "argos".into());
        let body = gmail::mcp_config_json(&exe);
        let path = paths::mcp_path();
        match secrets::write_private(
            &path,
            &serde_json::to_string_pretty(
                &serde_json::from_str::<serde_json::Value>(&body).unwrap_or(serde_json::json!({})),
            )
            .unwrap_or(body),
        ) {
            Ok(()) => {
                self.status = "gmail mcp config written".into();
                self.log_event(
                    "system",
                    &format!("Wrote {}. Launch with `argos mcp gmail`.", path.display()),
                );
            }
            Err(err) => {
                self.status = "gmail mcp write failed".into();
                self.log_event("system", &format!("mcp write failed: {err}"));
            }
        }
    }

    fn save_settings_fields(&mut self) {
        self.settings.searx_url = self.field_value("searx_url");
        self.settings.report_dir = self.field_value("report_dir");
        match self.settings.save() {
            Ok(()) => {
                self.status = "settings saved".into();
                self.log_event("system", "settings saved");
            }
            Err(err) => {
                self.status = "settings save failed".into();
                self.log_event("system", &format!("settings save failed: {err}"));
            }
        }
    }

    fn record_voice(&mut self) {
        let Some(secret) = self.auth.voice.clone().or_else(|| self.auth.text.clone()) else {
            self.status = "voice provider is not set".into();
            self.log_event(
                "system",
                "voice capture needs a provider that implements /audio/transcriptions",
            );
            return;
        };
        self.status = "listening".into();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            match record_wav().await {
                Err(err) => {
                    let _ = tx.send(AppMsg::Voice(Err(err)));
                }
                Ok(bytes) => {
                    let _ = tx.send(AppMsg::Note(format!("captured {} bytes", bytes.len())));
                    match provider::transcribe(&secret, &bytes).await {
                        Ok(text) => {
                            let _ = tx.send(AppMsg::Voice(Ok(text)));
                        }
                        Err(err) => {
                            let _ = tx.send(AppMsg::Voice(Err(err.to_string())));
                        }
                    }
                }
            }
        });
    }

    fn log_event(&mut self, kind: &str, text: &str) {
        let at = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            self.log.push(LogEntry {
                at: at.clone(),
                kind: kind.to_string(),
                text: line.to_string(),
            });
        }
        if self.log.len() > 400 {
            let drain = self.log.len() - 400;
            self.log.drain(0..drain);
        }
    }
}

fn note_kind(text: &str) -> &'static str {
    let lower = text.to_lowercase();
    if lower.contains("search")
        || lower.contains("web_search")
        || lower.contains("news_search")
        || lower.contains("social_search")
        || lower.contains("lookup")
        || lower.contains("fetch_page")
        || lower.contains("public hits")
    {
        "search"
    } else if lower.contains("fail") || lower.contains("error") || lower.contains("refused") {
        "task"
    } else {
        "system"
    }
}

fn module_session(module: ModuleId) -> String {
    format!("module:{}", module.title().to_lowercase().replace(' ', "-"))
}

fn on_off(on: bool) -> &'static str {
    if on {
        "on"
    } else {
        "off"
    }
}

fn yes_no(on: bool) -> String {
    if on {
        "yes".into()
    } else {
        "no".into()
    }
}

fn field(key: &str, label: &str, value: String, secret: bool) -> Field {
    Field {
        key: key.into(),
        label: label.into(),
        value,
        secret,
    }
}

fn empty_fallback(value: &str, fallback: &str) -> String {
    if value.trim().is_empty() {
        fallback.into()
    } else {
        value.trim().into()
    }
}

pub fn provider_label(secret: Option<&ProviderSecret>) -> String {
    match secret {
        Some(secret) if !secret.model.is_empty() => format!("{} {}", secret.kind, secret.model),
        _ => "not signed in".into(),
    }
}

pub fn report_dir(settings: &SettingsFile) -> PathBuf {
    if !settings.report_dir.trim().is_empty() {
        return PathBuf::from(settings.report_dir.trim());
    }
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("reports")
}

fn memory_answer_material(app: &App, hits: &[brain::ScoredMemory]) -> String {
    let mut out = String::from(
        "FACTS FROM COMPLETED REPORTS. Answer only from these facts. Name the report each fact came from so the user can open it. Do not read the report files and do not start a search.\n",
    );
    for hit in hits {
        let source = hit
            .memory
            .report_id
            .as_deref()
            .and_then(|id| app.reports.iter().find(|report| report.id == id))
            .map(|report| report.title.as_str())
            .unwrap_or("completed report");
        out.push_str(&format!(
            "- {}\n  source: {source}\n",
            hit.memory.text.trim()
        ));
    }
    out
}

fn prior_report_material(reports: &[ReportMeta]) -> String {
    let mut out = String::from(
        "RELEVANT REPORTS ALREADY ON FILE. Answer from these reports. Do not open a new investigation.\n",
    );
    for report in reports {
        out.push_str(&format!("\n# {}\n", report.title));
        match std::fs::read_to_string(&report.path) {
            Ok(body) => {
                let clipped: String = body.chars().take(3500).collect();
                out.push_str(&clipped);
                if body.chars().count() > 3500 {
                    out.push_str("\n…\n");
                } else {
                    out.push('\n');
                }
            }
            Err(err) => {
                out.push_str(&format!("(could not read {}: {err})\n", report.path));
            }
        }
    }
    out
}

async fn distill_insight(
    secret: &argos_osint_core::secrets::ProviderSecret,
    report: &str,
    question: &str,
    answer: &str,
) -> Result<String, String> {
    if secret.base_url.trim().is_empty() || secret.model.trim().is_empty() {
        return Err("no provider".into());
    }
    let messages = vec![provider::ChatMessage {
        role: "user".into(),
        content: format!(
            "Summarize this report-chat exchange as one or two concise sentences.\n\
             Keep the new insight from the user's question and the answer.\n\
             Paraphrase. Do not quote the whole reply. No preamble and no bullet list.\n\
             The fact is about report \"{report}\".\n\n\
             Question:\n{question}\n\n\
             Answer:\n{answer}"
        ),
        tool_call_id: None,
        tool_calls: Vec::new(),
    }];
    let completion = provider::complete(secret, &messages, &[], |_| {})
        .await
        .map_err(|err| err.to_string())?;
    Ok(completion.content)
}

fn summarize_hits(hits: &[SearchHit]) -> String {
    if hits.is_empty() {
        return "No public hits.".into();
    }
    let mut out = String::new();
    for (i, hit) in hits.iter().take(8).enumerate() {
        out.push_str(&format!("{}. {} — {}\n", i + 1, hit.title, hit.url));
    }
    out
}

async fn record_wav() -> Result<Vec<u8>, String> {
    let path = std::env::temp_dir().join(format!("argos-voice-{}.wav", std::process::id()));
    let path_str = path.display().to_string();
    let attempts = [
        vec![
            "rec", "-q", "-r", "16000", "-c", "1", &path_str, "trim", "0", "5",
        ],
        vec![
            "ffmpeg",
            "-y",
            "-f",
            "avfoundation",
            "-i",
            ":0",
            "-t",
            "5",
            "-ac",
            "1",
            "-ar",
            "16000",
            &path_str,
        ],
    ];
    let mut last = "no recorder found".to_string();
    for args in attempts {
        let mut cmd = tokio::process::Command::new(args[0]);
        cmd.args(&args[1..])
            .kill_on_drop(true)
            .stdout(std::process::Stdio::null());
        match cmd.output().await {
            Ok(out) if out.status.success() && path.exists() => {
                let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
                let _ = std::fs::remove_file(&path);
                if bytes.len() < 64 {
                    return Err("recording was empty".into());
                }
                return Ok(bytes);
            }
            Ok(out) => last = format!("{}: {}", args[0], String::from_utf8_lossy(&out.stderr)),
            Err(err) => last = format!("{}: {err}", args[0]),
        }
    }
    let _ = std::fs::remove_file(&path);
    Err(format!("Voice capture needs sox (`rec`) or ffmpeg. {last}"))
}

pub async fn run(mut app: App) -> Result<()> {
    use crossterm::event::EventStream;
    use crossterm::execute;
    use crossterm::terminal::{
        disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
    };
    use futures_util::StreamExt;
    use ratatui::backend::CrosstermBackend;
    use ratatui::Terminal;
    use std::io::stdout;

    let mut inbox = app.take_inbox();
    app.spawn_hardware(false);
    enable_raw_mode()?;
    let mut out = stdout();
    execute!(
        out,
        EnterAlternateScreen,
        crossterm::event::EnableMouseCapture
    )?;
    let backend = CrosstermBackend::new(out);
    let mut terminal = Terminal::new(backend)?;
    let mut reader = EventStream::new();
    let _guard = RawRestorer;
    loop {
        terminal.draw(|frame| super::ui::draw(frame, &mut app))?;
        if app.quit {
            break;
        }
        tokio::select! {
            biased;
            msg = inbox.recv() => {
                if let Some(msg) = msg {
                    app.on_msg(msg);
                }
            }
            ev = reader.next() => {
                match ev {
                    Some(Ok(ev)) => { app.on_event(ev); }
                    Some(Err(_)) | None => break,
                }
            }
            _ = tokio::time::sleep(Duration::from_millis(200)) => app.tick(),
        }
        if app.quit {
            break;
        }
    }
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        crossterm::event::DisableMouseCapture
    )?;
    let _ = terminal.show_cursor();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use argos_osint_core::provider::SettingsFile;
    use argos_osint_core::secrets::AuthFile;
    use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    #[test]
    fn classic_frame_mentions_the_launcher_and_prompt() {
        let store = Store::memory().unwrap();
        let mut app = App::from_parts(store, SettingsFile::default(), AuthFile::default()).unwrap();
        app.hardware.cpu_name = "Test CPU".into();
        app.hardware.logical_cores = 8;
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| super::super::ui::draw(frame, &mut app))
            .unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(text.contains("Apps"), "{text}");
        assert!(text.contains("Case Desk"), "{text}");
        assert!(text.contains("Reports"), "{text}");
        assert!(text.contains("No reports yet"), "{text}");
        assert!(
            !text.contains("Reports  Brain") && !text.contains("Reports Brain"),
            "{text}"
        );
        assert!(text.contains("Ctrl+P"), "{text}");
        assert_eq!(super::super::slash_menu("/use").option_count(), 1);
        let chat = app.canvas_area;
        app.click(chat.x + 2, chat.y + 2);
        assert_eq!(app.focus, Focus::Canvas);
        assert_eq!(app.case_tab_hits.len(), 3);
        assert!(text.contains("System"), "{text}");
        assert!(!text.contains("Search Log"), "{text}");
        let brain = app.case_tab_hits[1];
        app.click(brain.x + 1, brain.y);
        assert_eq!(app.case_page, CasePage::Brain);

        app.open_module(ModuleId::Brain);
        terminal
            .draw(|frame| super::super::ui::draw(frame, &mut app))
            .unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(text.contains("Memories"), "{text}");
        assert!(text.contains("Enter views a fact"), "{text}");
        assert!(!text.contains("No reports yet"), "{text}");
        app.brain_card = true;
        terminal
            .draw(|frame| super::super::ui::draw(frame, &mut app))
            .unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(text.contains("Memory"), "{text}");
        assert!(text.contains("Close"), "{text}");
        assert!(!text.contains("Edit"), "{text}");
    }

    #[test]
    fn case_page_all_includes_network() {
        let pages = CasePage::all();
        assert_eq!(pages.len(), 3);
        assert!(pages.contains(&CasePage::Network));
        assert_eq!(CasePage::Network.title(), "Network");
        assert_eq!(
            pages.map(|p| p.title()),
            ["Desk", "Brain", "Network"]
        );
    }

    #[test]
    fn slash_network_opens_network_page() {
        let store = Store::memory().unwrap();
        let mut app = App::from_parts(store, SettingsFile::default(), AuthFile::default()).unwrap();
        app.run_slash("/network");
        assert_eq!(app.case_page, CasePage::Network);
        assert_eq!(app.focus, Focus::Graph);
    }

    #[test]
    fn tna_glyphs_by_kind() {
        use argos_osint_core::tna::TnaNodeKind;
        assert_eq!(tna_glyph_for_kind(TnaNodeKind::Domain), "◆");
        assert_eq!(tna_glyph_for_kind(TnaNodeKind::Ip), "◆");
        assert_eq!(tna_glyph_for_kind(TnaNodeKind::Topic), "▲");
        assert_eq!(tna_glyph_for_kind(TnaNodeKind::Org), "▲");
        assert_eq!(tna_glyph_for_kind(TnaNodeKind::Person), "●");
        assert_eq!(tna_glyph_for_kind(TnaNodeKind::Handle), "●");
        assert_eq!(tna_glyph_for_kind(TnaNodeKind::Email), "●");
        assert_eq!(tna_glyph_for_kind(TnaNodeKind::Doc), "□");
    }

    #[test]
    fn tna_hub_threshold_drops_to_eight_when_max_degree_low() {
        let nodes = vec![
            TnaNode {
                id: "a".into(),
                label: "a".into(),
                kind: TnaNodeKind::Domain,
                cluster: TnaCluster::Infrastructure,
                mentions: 1,
                degree: 10,
                x: 0.0,
                y: 0.0,
            },
            TnaNode {
                id: "b".into(),
                label: "b".into(),
                kind: TnaNodeKind::Handle,
                cluster: TnaCluster::Identity,
                mentions: 1,
                degree: 3,
                x: 0.0,
                y: 0.0,
            },
        ];
        assert_eq!(tna_hub_threshold(&nodes), 8);
        let mut high = nodes.clone();
        high[0].degree = 20;
        assert_eq!(tna_hub_threshold(&high), 15);
    }

    #[test]
    fn tna_ego_collapse_counts_far_neighbors() {
        // Hub H connected to focus F and 8 peripheral nodes (degree 9).
        // From F, ego1 = {F,H}; far neighbors of H collapse when threshold=8.
        let mut nodes = vec![TnaNode {
            id: "F".into(),
            label: "focus".into(),
            kind: TnaNodeKind::Person,
            cluster: TnaCluster::Identity,
            mentions: 1,
            degree: 1,
            x: 0.2,
            y: 0.5,
        }, TnaNode {
            id: "H".into(),
            label: "hub".into(),
            kind: TnaNodeKind::Domain,
            cluster: TnaCluster::Infrastructure,
            mentions: 1,
            degree: 9,
            x: 0.5,
            y: 0.5,
        }];
        let mut edges = vec![argos_osint_core::tna::TnaEdge {
            from: "F".into(),
            to: "H".into(),
            weight: 1,
        }];
        for i in 0..8 {
            let id = format!("p{i}");
            nodes.push(TnaNode {
                id: id.clone(),
                label: id.clone(),
                kind: TnaNodeKind::Handle,
                cluster: TnaCluster::Identity,
                mentions: 1,
                degree: 1,
                x: 0.8,
                y: 0.1 * i as f64,
            });
            edges.push(argos_osint_core::tna::TnaEdge {
                from: "H".into(),
                to: id,
                weight: 1,
            });
        }
        let snap = TnaSnapshot {
            scope: argos_osint_core::tna::TnaScope::Collection,
            title: "TNA · test".into(),
            nodes,
            edges,
            clusters: vec![],
            anchors: vec![],
            gaps: vec![],
            built_at: "t".into(),
        };
        let expanded = HashSet::new();
        let (visible, supers) = App::tna_ego_visible_indices(&snap, "F", &expanded);
        assert!(visible.contains(&0), "focus visible: {visible:?}");
        assert!(visible.contains(&1), "hub visible: {visible:?}");
        assert_eq!(supers.len(), 1, "one supernode: {supers:?}");
        assert_eq!(supers[0].0, "H");
        assert_eq!(supers[0].1, 8);
        assert_eq!(visible.len(), 2, "peripherals collapsed: {visible:?}");

        let mut expanded = HashSet::new();
        expanded.insert("H".into());
        let (visible2, supers2) = App::tna_ego_visible_indices(&snap, "F", &expanded);
        assert!(supers2.is_empty());
        assert_eq!(visible2.len(), 10, "all expanded: {visible2:?}");
    }

    #[test]
    fn tna_view_key_v_cycles() {
        let store = Store::memory().unwrap();
        let mut app = App::from_parts(store, SettingsFile::default(), AuthFile::default()).unwrap();
        app.run_slash("/network");
        assert_eq!(app.tna_view, TnaView::Graph);
        assert!(app.tna_selected_real_idx().is_none() || app.tna_selected_item().is_some());
        assert!(app.tna_selected_super_hub().is_none());
        let key = KeyEvent::new(KeyCode::Char('v'), KeyModifiers::NONE);
        app.on_tna_key(key);
        assert_eq!(app.tna_view, TnaView::Outline);
        app.on_tna_key(key);
        assert_eq!(app.tna_view, TnaView::Table);
        app.on_tna_key(key);
        assert_eq!(app.tna_view, TnaView::Graph);
    }

    #[test]
    fn system_log_hides_configuration_errors_from_the_desk_and_reports() {
        let store = Store::memory().unwrap();
        let mut app = App::from_parts(store, SettingsFile::default(), AuthFile::default()).unwrap();
        let marker = "sysctl-config-UNIQUE-9f3a";
        app.pending_reports.push(PendingReport {
            case_id: "case-1".into(),
            title: "who is ada".into(),
            failed: None,
        });
        app.on_msg(AppMsg::Research {
            case_id: "case-1".into(),
            result: Err(marker.into()),
        });
        app.on_msg(AppMsg::Turn(TurnEvent::Failed(format!("api {marker}"))));
        app.on_msg(AppMsg::Models(Err(format!("provider {marker}"))));

        let chat = app
            .transcript()
            .iter()
            .map(|line| line.body.clone())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!chat.contains(marker), "{chat}");
        assert!(chat.contains("System log"), "{chat}");
        assert!(app.log.iter().any(|entry| {
            entry.text.contains(marker) && entry.at.len() == "2026-09-24 15:04:01".len()
        }));
        assert!(app
            .report_rows()
            .iter()
            .any(|row| row.status() == "failed" && row.title() == "who is ada"));

        let backend = TestBackend::new(140, 40);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| super::super::ui::draw(frame, &mut app))
            .unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(!text.contains(marker), "{text}");
        assert!(text.contains("failed"), "{text}");
        assert!(!text.contains("Search Log"), "{text}");

        app.open_module(ModuleId::System);
        terminal
            .draw(|frame| super::super::ui::draw(frame, &mut app))
            .unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(text.contains(marker), "{text}");
        assert!(text.contains("Log"), "{text}");
        assert!(text.contains("Hardware"), "{text}");
        assert!(text.contains("Settings"), "{text}");
        assert_eq!(app.system_tab_hits.len(), 3);
        assert_eq!(app.system_page, SystemPage::Log);
    }

    #[test]
    fn free_route_lists_concrete_models_and_roles_are_separate() {
        use argos_osint_core::provider::ListedModel;
        use argos_osint_core::secrets::ProviderSecret;

        let store = Store::memory().unwrap();
        let mut app = App::from_parts(store, SettingsFile::default(), AuthFile::default()).unwrap();
        app.auth.text = Some(ProviderSecret {
            kind: "openrouter".into(),
            base_url: "https://openrouter.ai/api/v1".into(),
            model: "openai/gpt-4.1".into(),
            api_key: None,
            stt_model: None,
            device: None,
        });
        app.settings.model = "openai/gpt-4.1".into();
        app.catalog = vec![
            ListedModel {
                id: "openrouter/free".into(),
                name: "Free Models Router".into(),
                free: false,
            },
            ListedModel {
                id: "meta-llama/llama-3.2-3b-instruct:free".into(),
                name: "Llama 3.2 3B".into(),
                free: true,
            },
        ];
        app.remote_models = vec![
            "openrouter/free".into(),
            "meta-llama/llama-3.2-3b-instruct:free".into(),
            "openai/gpt-4.1".into(),
        ];
        app.model_picker = true;
        app.model_sel = app
            .filtered_model_choices()
            .iter()
            .position(|(id, _)| id == "openrouter/free")
            .unwrap();
        app.on_event(key(KeyCode::Enter));
        assert!(app.free_picker);
        assert_eq!(app.settings.model, "openai/gpt-4.1");
        assert_eq!(app.filtered_free_models().len(), 1);

        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| super::super::ui::draw(frame, &mut app))
            .unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(text.contains("Free models"), "{text}");
        assert!(text.contains("Llama 3.2 3B"), "{text}");
        assert!(text.contains("at random"), "{text}");

        app.on_event(key(KeyCode::Enter));
        assert!(!app.free_picker);
        assert_eq!(app.settings.model, "meta-llama/llama-3.2-3b-instruct:free");
        assert!(app.settings.writer_model.is_empty());

        app.model_target = ModelTarget::Tool;
        app.model_picker = true;
        app.model_sel = app
            .filtered_model_choices()
            .iter()
            .position(|(id, _)| id == "openai/gpt-4.1")
            .unwrap();
        app.on_event(key(KeyCode::Enter));
        assert_eq!(app.settings.tool_model, "openai/gpt-4.1");
        assert_eq!(app.settings.model, "meta-llama/llama-3.2-3b-instruct:free");

        app.open_module(ModuleId::Providers);
        let labels: Vec<_> = app
            .fields
            .iter()
            .map(|field| field.label.as_str())
            .collect();
        assert!(labels.contains(&"Connection"));
        assert!(labels.contains(&"Writer model"));
        assert!(labels.contains(&"Tool model"));
    }

    fn key(code: KeyCode) -> Event {
        Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    #[test]
    fn plus_opens_scope_and_escape_restores_the_query() {
        let store = Store::memory().unwrap();
        let mut app = App::from_parts(store, SettingsFile::default(), AuthFile::default()).unwrap();
        app.prompt = "who is ada".into();
        app.cursor = app.prompt.chars().count();
        app.on_event(key(KeyCode::Char('+')));
        let scope = app.scope.as_ref().expect("scope card");
        assert!(scope.facts);
        assert!(scope.domain);
        assert_eq!(scope.query, "who is ada");
        assert!(app.prompt.is_empty());
        assert!(app.pending_reports.is_empty());

        app.on_event(key(KeyCode::Char(' ')));
        assert!(!app.scope.as_ref().unwrap().facts);
        assert!(app.settings.facts);

        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| super::super::ui::draw(frame, &mut app))
            .unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(text.contains("Facts"), "{text}");
        assert!(text.contains("Identity"), "{text}");
        assert!(text.contains("Space toggles"), "{text}");

        app.on_event(key(KeyCode::Esc));
        assert!(app.scope.is_none());
        assert_eq!(app.prompt, "who is ada");
        assert!(app.pending_reports.is_empty());
    }

    #[test]
    fn start_case_worker_opens_scope_before_research() {
        let store = Store::memory().unwrap();
        let mut app = App::from_parts(store, SettingsFile::default(), AuthFile::default()).unwrap();
        app.confirm_query = Some("example.com".into());
        app.confirm_sel = 0;
        app.on_event(key(KeyCode::Enter));
        assert!(app.confirm_query.is_none());
        assert_eq!(
            app.scope.as_ref().map(|scope| scope.query.as_str()),
            Some("example.com")
        );
        assert!(app.pending_reports.is_empty());
        app.on_event(key(KeyCode::Down));
        app.on_event(key(KeyCode::Char(' ')));
        assert!(!app.scope.as_ref().unwrap().web);
        assert!(app.settings.web);
    }
}

struct RawRestorer;
impl Drop for RawRestorer {
    fn drop(&mut self) {
        let _ = crossterm::terminal::disable_raw_mode();
        let _ = crossterm::execute!(
            std::io::stdout(),
            crossterm::terminal::LeaveAlternateScreen,
            crossterm::event::DisableMouseCapture
        );
    }
}
