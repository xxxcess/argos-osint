//! Session shell. The bottom prompt always talks to the view that is open:
//! the desk, the selected case, or the module on the canvas.

#[path = "case_workspace.rs"]
pub mod case_workspace;
use case_workspace::{CaseView, CaseWorkspace};

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;

use anyhow::Result;
use argos_osint_core::agent::{self, HistMsg, TurnEvent, TurnInput};
use argos_osint_core::brain::{self, Memory};
use argos_osint_core::gmail::GmailConfig;
use argos_osint_core::hardware::{self, HardwareProfile};
use argos_osint_core::paths::{self, db_label};
use argos_osint_core::prompt::{self, Intent};
use argos_osint_core::provider::{self, SettingsFile};
use argos_osint_core::report::{self, ReportMeta};
use argos_osint_core::search::{SearchHit, SourcePlan};
use argos_osint_core::secrets::{AuthFile, ProviderSecret};
use argos_osint_core::session::{self, Case};
use argos_osint_core::store::{ChatLine, Store};
use argos_osint_core::tna::{
    self, desk_key, report_key, TnaCluster, TnaNode, TnaNodeKind, TnaSnapshot,
};
use chrono::Local;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers, MouseEventKind};
use ratatui::layout::Rect;
use ratatui::widgets::{ListState, ScrollbarState, TableState};
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
            Self::Providers => "Provider accounts and Writer / Tools models",
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
    Investigation,
}

impl CasePage {
    pub fn all() -> [Self; 2] {
        [Self::Closed, Self::Brain]
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Closed => "Desk",
            Self::Brain => "Brain",
            Self::Network => "Network",
            Self::Investigation => "Investigation",
        }
    }
}

/// Maximum number of connected entities drawn as detail boxes.
pub const TNA_GRAPH_BOX_BUDGET: usize = 16;
pub const TNA_DETAIL_HOPS: u32 = 5;

/// A real entity in the report table or its connected detail boxes.
#[derive(Clone, Debug, PartialEq)]
pub enum TnaDisplayItem {
    Real { idx: usize },
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

/// Explicit first actions and source permissions for a case investigation.
#[derive(Clone, Debug)]
pub struct ScopeDraft {
    pub existing_case: Option<String>,
    pub include_evidence: bool,
    pub allow_sensitive: bool,
    pub allow_active: bool,
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
    Case(Case),
}

impl ReportRow {
    pub fn title(&self) -> &str {
        match self {
            Self::Pending { title, .. } => title,
            Self::Completed(report) => &report.title,
            Self::Case(case) => &case.title,
        }
    }

    pub fn case_id(&self) -> Option<&str> {
        match self {
            Self::Pending { case_id, .. } => Some(case_id),
            Self::Completed(report) => report.case_id.as_deref(),
            Self::Case(case) => Some(&case.id),
        }
    }

    pub fn status(&self) -> &'static str {
        match self {
            Self::Pending {
                failed: Some(_), ..
            } => "failed",
            Self::Pending { .. } => "pending",
            Self::Completed(_) => "completed",
            Self::Case(_) => "case",
        }
    }

    pub fn when(&self) -> Option<String> {
        match self {
            Self::Completed(report) => Some(report::short_when(&report.created_at)),
            _ => None,
        }
    }
}

/// Account credentials and role assignments are separate destinations.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ProviderPage {
    Grok,
    Openai,
    Openrouter,
    Models,
    Osint,
    Research,
}

impl ProviderPage {
    pub fn all() -> [Self; 6] {
        [
            Self::Grok,
            Self::Openai,
            Self::Openrouter,
            Self::Models,
            Self::Osint,
            Self::Research,
        ]
    }
    pub fn title(self) -> &'static str {
        match self {
            Self::Grok => "Grok",
            Self::Openai => "OpenAI",
            Self::Openrouter => "OpenRouter",
            Self::Models => "Models",
            Self::Osint => "Sources",
            Self::Research => "Research",
        }
    }
    pub fn account(self) -> Option<&'static str> {
        match self {
            Self::Grok => Some("grok"),
            Self::Openai => Some("openai-chatgpt"),
            Self::Openrouter => Some("openrouter"),
            _ => None,
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
    /// Table master–detail: details pane (list uses Focus::Graph).
    TableDetail,
}

/// Session-only presentations of the same persisted report graph.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TnaLayout {
    #[default]
    Cockpit,
    Clusters,
    Path,
    Matrix,
    Ribbon,
}

impl TnaLayout {
    pub const ALL: [Self; 5] = [
        Self::Cockpit,
        Self::Clusters,
        Self::Path,
        Self::Matrix,
        Self::Ribbon,
    ];
    pub fn title(self) -> &'static str {
        match self {
            Self::Cockpit => "Cockpit",
            Self::Clusters => "Clusters",
            Self::Path => "Path",
            Self::Matrix => "Matrix",
            Self::Ribbon => "Ribbon",
        }
    }
    pub fn index(self) -> usize {
        Self::ALL.iter().position(|v| *v == self).unwrap_or(0)
    }
}

#[derive(Clone, Debug, Default)]
pub struct TnaAnswer {
    pub question: String,
    pub answer: String,
    pub pending: bool,
    pub error: Option<String>,
    pub filed: bool,
}

#[derive(Clone, Debug)]
pub struct TnaPath {
    pub nodes: Vec<String>,
    pub strength: u32,
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
    CaseDrafted {
        case_id: String,
        result: Result<ReportMeta, String>,
    },
    CaseDataPlanned {
        generation: u64,
        result: Result<argos_osint_core::store::CaseDataPlan, String>,
    },
    CaseDataApplied {
        plan: argos_osint_core::store::CaseDataPlan,
        result: Result<(), String>,
    },
    DeskCaseReady {
        case_id: String,
        generation: u64,
        result: Result<argos_osint_core::investigation::CaseProjection, String>,
    },
    CaseReady {
        case_id: String,
        generation: u64,
        result: Result<argos_osint_core::investigation::CaseProjection, String>,
    },
    CaseSource {
        case_id: String,
        generation: u64,
        result: Result<String, String>,
    },
    ReviewedRetrieved {
        generation: u64,
        query: String,
        result: Result<
            (
                Vec<argos_osint_core::evidence::PassageHit>,
                Vec<argos_osint_core::evidence::Finding>,
            ),
            String,
        >,
    },
    ReportSource {
        report_id: String,
        generation: u64,
        version: Option<i64>,
        result: Result<String, String>,
    },
    ResearchJob(argos_osint_core::research::ResearchJob),
    ResearchProgress(argos_osint_core::research::ResearchJob),
    ToolManaged {
        name: String,
        result: Result<Option<String>, String>,
    },
    ResearchTest {
        name: String,
        state: argos_osint_core::research::Readiness,
        detail: String,
    },
    EvidenceSurface {
        report_id: Option<String>,
        title: String,
        result: Result<String, String>,
        scope: argos_osint_core::evidence::EvidenceScope,
    },
    ReportUpdated(Result<ReportMeta, String>),
    Coverage {
        report_id: Option<String>,
        result: Result<Vec<argos_osint_core::evidence::CoverageRow>, String>,
    },
    OpenPassage(Result<Option<argos_osint_core::evidence::PassageHit>, String>),
    #[cfg(test)]
    Turn(TurnEvent),
    ScopedTurn {
        generation: u64,
        report_id: Option<String>,
        event: TurnEvent,
    },
    Hardware(HardwareProfile),
    Note(String),
    Search {
        query: String,
        result: Result<Vec<SearchHit>, String>,
    },
    ModelList {
        kind: String,
        generation: u64,
        draft: bool,
        result: Result<Vec<provider::ListedModel>, String>,
    },
    SubscriptionCheck(Result<String, String>),
    SubscriptionProgress(String),
    GrokSubscriptionProgress {
        generation: u64,
        line: String,
    },
    GrokSubscriptionCheck {
        generation: u64,
        result: Result<Vec<provider::ListedModel>, String>,
    },
    Voice(Result<String, String>),
    #[cfg(test)]
    Research {
        case_id: String,
        result: Result<(String, Option<ReportMeta>), String>,
    },
    Insight {
        report_id: String,
        fact: String,
        generation: u64,
    },
    TnaReady {
        targeted: bool,
        report_id: Option<String>,
        scope: Option<argos_osint_core::evidence::EvidenceScope>,
        result: Result<TnaSnapshot, String>,
    },
}

pub struct App {
    pub desk_cases: HashMap<String, argos_osint_core::investigation::CaseProjection>,
    desk_generations: HashMap<String, u64>,
    pub desk_transcript: bool,
    pub desk_refreshing: std::collections::HashSet<String>,
    pub desk_pane: usize,
    pub desk_row: usize,
    pub investigation: Option<CaseWorkspace>,
    case_generation: u64,
    case_source_generation: u64,
    case_cancel: Arc<AtomicBool>,
    desk_return_scope: argos_osint_core::evidence::EvidenceScope,
    reviewed_recommendations: Vec<argos_osint_core::evidence::Finding>,
    pub research_phase: argos_osint_core::research::ResearchPhase,
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
    /// Suggested question waiting for an explicit scope review.
    pub confirm_query: Option<String>,
    pub confirm_sel: usize,
    /// Investigation scope and unchecked first actions.
    pub scope: Option<ScopeDraft>,
    /// Completed report waiting on the open-workspace confirmation popup.
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
    /// Active dedicated report network workspace; no transcript is loaded.
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
    /// Stock List selection (kept in sync with `brain_sel`).
    pub brain_list_state: ListState,
    /// Memory-list scrollbar (ITEM_HEIGHT = 1).
    pub brain_list_scroll: ScrollbarState,
    /// Hit area for Brain list (wheel scroll).
    pub brain_list_area: Rect,
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
    pub provider_picker: Option<bool>,
    pub provider_choice: usize,
    #[cfg(test)]
    test_config_home: tempfile::TempDir,
    pub provider_checks: HashMap<String, String>,
    pub provider_draft_checks: HashMap<String, String>,
    pub model_catalogs: HashMap<String, Vec<provider::ListedModel>>,
    catalog_generation: HashMap<String, u64>,
    pub grok_subscription_status: String,
    pub grok_subscription_pending: bool,
    pub grok_subscription_instructions: Vec<String>,
    pub subscription_status: String,
    pub subscription_pending: bool,
    pub subscription_instructions: Vec<String>,
    pub provider_field_hits: Vec<(usize, Rect)>,
    pub provider_advanced: bool,
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
    pub tna_layout: TnaLayout,
    pub tna_answer: Option<TnaAnswer>,
    pub tna_from: Option<String>,
    pub tna_to: Option<String>,
    pub tna_hop_sel: usize,
    pub tna_path_sel: usize,
    pub tna_matrix_coverage: bool,
    pub coverage_rows: Vec<argos_osint_core::evidence::CoverageRow>,
    pub tna_matrix_row: usize,
    pub tna_matrix_col: usize,
    pub tna_ribbon_pos: usize,
    pub tna_show_rejected: bool,
    pub tna_find_editing: bool,
    pub tool_plan: Option<(String, String, argos_osint_core::tool_manager::InstallPlan)>,
    pub research_sel: usize,
    pub research_jobs: Vec<argos_osint_core::research::ResearchJob>,
    pub research_active: usize,
    pub pending_case_data: Option<argos_osint_core::store::CaseDataPlan>,
    pub case_data_generation: u64,
    pub case_data_busy: Option<String>,
    pub case_pending_work: HashMap<String, usize>,
    research_queue: Option<argos_osint_core::research::ResearchQueue>,
    pub recommendations: Vec<argos_osint_core::evidence::PassageHit>,
    pub evidence_scope: argos_osint_core::evidence::EvidenceScope,
    pub workspace_reading: bool,
    pub report_read_line: usize,
    pub originating_question: String,
    pub selected_passage: Option<(String, usize, usize, i64)>,
    pub report_source_version: Option<i64>,
    pub report_read_source: Option<String>,
    report_source_generation: u64,
    desk_return_scroll: usize,
    desk_return_prompt: String,
    pub tna_source: String,
    pub tna_source_error: Option<String>,
    pub tna_tab_hits: Vec<Rect>,
    pub tna_ledger_sel: usize,
    pub tna_answer_scroll: u16,
    turn_generation: u64,
    pending_tna_insights: HashMap<u64, (String, String)>,
    tna_path_cache: RefCell<Option<(String, String, u64, Vec<TnaPath>)>>,
    pub tna_desk: Option<TnaSnapshot>,
    pub tna_report: Option<TnaSnapshot>,
    pub tna_find: Option<String>,
    pub tna_sel: usize,
    pub tna_rebuilding: bool,
    /// Hub node ids whose far neighbors are expanded (not collapsed to ▣×N).
    /// Egocentric pin for Graph collapse (node id).
    pub tna_focus_id: Option<String>,
    /// Stock Table selection (kept in sync with `tna_sel`).
    pub tna_table_state: TableState,
    /// Initiative-list scrollbar (ITEM_HEIGHT = 1).
    pub tna_table_scroll: ScrollbarState,
    /// Detail-pane context scroll offset (lines); ego boxes stay fixed.
    pub tna_detail_scroll: usize,
    /// Detail-pane scrollbar state.
    /// Hit areas for Table master–detail (wheel / focus).
    pub tna_table_list_area: Rect,
    pub tna_table_detail_area: Rect,
    /// File-backed DB path so TNA rebuilds can run on a second SQLite connection.
    /// Tests use an in-memory store and leave this `None` (sync rebuild).
    tna_db_path: Option<PathBuf>,
    tna_rebuild_gen: u64,
    tna_pending_report: Option<String>,
    pub tna_path_search_limited: Cell<bool>,
}

impl App {
    pub fn boot() -> Result<Self> {
        paths::ensure_home()?;
        let store = Store::open(&paths::db_path())?;
        store.ensure_session("desk", "Desk", "desk")?;
        for module in ModuleId::all() {
            if module != ModuleId::Cases {
                store.ensure_session(&module_session(module), module.title(), "module")?;
            }
        }
        let (settings, config_error) = match SettingsFile::load() {
            Ok(settings) => (settings, None),
            Err(err) => (SettingsFile::default(), Some(err.to_string())),
        };
        let auth = AuthFile::load()?;
        let mut app = Self::from_parts(store, settings, auth)?;
        app.tna_db_path = Some(paths::db_path());
        app.store.recover_jobs()?;
        app.research_jobs = app.store.jobs()?;
        app.research_queue = Some(argos_osint_core::research::ResearchQueue::new(
            paths::db_path(),
            4,
        ));
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
            provider_page: ProviderPage::Models,
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
            brain_list_state: ListState::default().with_selected(Some(0)),
            brain_list_scroll: ScrollbarState::new(0),
            brain_list_area: Rect::default(),
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
            provider_picker: None,
            provider_choice: 0,
            #[cfg(test)]
            test_config_home: tempfile::tempdir()?,
            provider_checks: HashMap::new(),
            provider_draft_checks: HashMap::new(),
            model_catalogs: HashMap::new(),
            catalog_generation: HashMap::new(),
            grok_subscription_status: "Not checked · Sign in or Check existing login".into(),
            grok_subscription_pending: false,
            grok_subscription_instructions: Vec::new(),
            subscription_status: "Not checked · Sign in or Check existing login".into(),
            subscription_pending: false,
            subscription_instructions: Vec::new(),
            provider_field_hits: Vec::new(),
            provider_advanced: false,
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
            tna_layout: TnaLayout::Cockpit,
            tna_answer: None,
            tna_from: None,
            tna_to: None,
            tna_hop_sel: 0,
            tna_path_sel: 0,
            tna_matrix_coverage: false,
            coverage_rows: Vec::new(),
            tna_matrix_row: 0,
            tna_matrix_col: 0,
            tna_ribbon_pos: 0,
            tna_show_rejected: false,
            tna_find_editing: false,
            tool_plan: None,
            investigation: None,
            case_generation: 0,
            case_source_generation: 0,
            case_cancel: Arc::new(AtomicBool::new(false)),
            desk_cases: HashMap::new(),
            desk_generations: HashMap::new(),
            desk_transcript: false,
            desk_refreshing: Default::default(),
            desk_pane: 0,
            desk_row: 0,
            desk_return_scope: Default::default(),
            reviewed_recommendations: Vec::new(),
            research_phase: Default::default(),
            research_sel: 0,
            research_jobs: Vec::new(),
            research_active: 0,
            pending_case_data: None,
            case_data_generation: 0,
            case_data_busy: None,
            case_pending_work: HashMap::new(),
            research_queue: None,
            recommendations: Vec::new(),
            evidence_scope: argos_osint_core::evidence::EvidenceScope::Collection,
            workspace_reading: false,
            report_read_line: 0,
            originating_question: String::new(),
            selected_passage: None,
            report_source_version: None,
            report_read_source: None,
            report_source_generation: 0,
            desk_return_scroll: 0,
            desk_return_prompt: String::new(),
            tna_source: String::new(),
            tna_source_error: None,
            tna_tab_hits: Vec::new(),
            tna_ledger_sel: 0,
            tna_answer_scroll: 0,
            turn_generation: 0,
            pending_tna_insights: HashMap::new(),
            tna_path_cache: RefCell::new(None),
            tna_desk: None,
            tna_report: None,
            tna_find: None,
            tna_sel: 0,
            tna_rebuilding: false,
            tna_focus_id: None,
            tna_table_state: TableState::default().with_selected(Some(0)),
            tna_table_scroll: ScrollbarState::new(0),
            tna_detail_scroll: 0,
            tna_table_list_area: Rect::default(),
            tna_table_detail_area: Rect::default(),
            tna_db_path: None,
            tna_rebuild_gen: 0,
            tna_pending_report: None,
            tna_path_search_limited: Cell::new(false),
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

    fn attach_research_progress(&self) {
        if let Some(queue) = &self.research_queue {
            let mut rx = queue.subscribe();
            let tx = self.tx.clone();
            tokio::spawn(async move {
                loop {
                    match rx.recv().await {
                        Ok(job) => {
                            if tx.send(AppMsg::ResearchProgress(job)).is_err() {
                                break;
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(_) => break,
                    }
                }
            });
        }
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
        self.sync_brain_list_ui();
        self.desk_cases
            .retain(|id, _| self.cases.iter().any(|c| &c.id == id));
        let missing = self
            .cases
            .iter()
            .filter(|c| {
                !self.desk_cases.contains_key(&c.id) && !self.desk_generations.contains_key(&c.id)
            })
            .map(|c| c.id.clone())
            .collect::<Vec<_>>();
        for id in missing {
            self.refresh_desk_case(&id);
        }
        Ok(())
    }

    pub fn desk_projection(&self) -> argos_osint_core::investigation::DeskProjection {
        use argos_osint_core::evidence::EvidenceScope;
        let mut cases = self
            .desk_cases
            .iter()
            .filter(|(id, _)| match &self.evidence_scope {
                EvidenceScope::Case(case) => *id == case,
                EvidenceScope::Report(report) => self
                    .reports
                    .iter()
                    .any(|r| &r.id == report && r.case_id.as_ref() == Some(*id)),
                EvidenceScope::Reports(reports) => self
                    .reports
                    .iter()
                    .any(|r| reports.contains(&r.id) && r.case_id.as_ref() == Some(*id)),
                _ => true,
            })
            .map(|(id, data)| (id.clone(), data.clone()))
            .collect::<Vec<_>>();
        cases.sort_by_key(|(id, _)| id.clone());
        argos_osint_core::investigation::DeskProjection::build(&cases)
    }
    fn refresh_desk_case(&mut self, id: &str) {
        let generation = self
            .desk_generations
            .get(id)
            .copied()
            .unwrap_or(0)
            .wrapping_add(1);
        self.desk_generations.insert(id.into(), generation);
        self.desk_refreshing.insert(id.into());
        let mut configs = self.settings.research.clone();
        if let Some(c) = configs.get_mut("shodan") {
            if self
                .auth
                .research
                .get(&c.secret_ref)
                .is_none_or(|s| s.is_empty())
            {
                c.readiness = argos_osint_core::research::Readiness::MissingCredentials;
            }
        }

        if let Some(path) = self.tna_db_path.clone() {
            let id = id.to_string();
            let tx = self.tx.clone();
            argos_osint_core::workers::spawn_blocking(move || {
                let result = Store::open(&path)
                    .and_then(|s| {
                        s.case_projection_with_research(&id, &AtomicBool::new(false), &configs)
                    })
                    .map_err(|e| e.to_string());
                let _ = tx.send(AppMsg::DeskCaseReady {
                    case_id: id,
                    generation,
                    result,
                });
            });
        } else {
            let result = self
                .store
                .case_projection_with_research(id, &AtomicBool::new(false), &configs)
                .map_err(|e| e.to_string());
            self.on_msg(AppMsg::DeskCaseReady {
                case_id: id.into(),
                generation,
                result,
            });
        }
    }
    fn on_operations_desk_key(&mut self, key: KeyEvent) -> bool {
        let desk = self.desk_projection();
        let len = match self.desk_pane {
            0 => desk.cards.len(),
            1 => desk.queue.len(),
            _ => desk.questions.len(),
        };
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                self.desk_row = (self.desk_row + 1).min(len.saturating_sub(1))
            }
            KeyCode::Char('k') | KeyCode::Up => self.desk_row = self.desk_row.saturating_sub(1),
            KeyCode::Char('g') => {
                self.desk_pane = 2;
                self.desk_row = 0;
            }
            KeyCode::Char('q') => {
                self.desk_pane = 1;
                self.desk_row = 0;
            }
            KeyCode::Char('w') => {
                self.desk_pane = 0;
                self.desk_row = 0;
            }
            KeyCode::Char('\\') => self.desk_transcript = !self.desk_transcript,
            KeyCode::Enter => {
                let target = match self.desk_pane {
                    0 => desk.cards.get(self.desk_row).map(|c| {
                        (
                            c.case_id.clone(),
                            c.lead_id.clone(),
                            c.action.clone(),
                            c.gap_id.clone(),
                            c.kind == argos_osint_core::investigation::NextWorkKind::Product,
                        )
                    }),
                    1 => desk
                        .queue
                        .get(self.desk_row)
                        .map(|r| (r.case_id.clone(), None, None, None, false)),
                    _ => desk.questions.get(self.desk_row).map(|g| {
                        (
                            g.case_id.clone(),
                            Some(g.entity_id.clone()),
                            if g.kind == argos_osint_core::investigation::GapKind::Uncollected {
                                g.action.clone()
                            } else {
                                None
                            },
                            Some(g.id.clone()),
                            false,
                        )
                    }),
                };
                if let Some((id, lead, action, gap, product)) = target {
                    self.open_investigation(&id);
                    if let Some(w) = self.investigation.as_mut() {
                        if let Some(data) = self.desk_cases.get(&id) {
                            w.data = data.clone();
                            w.snapshot = data.snapshot.clone();
                        }
                        if lead.is_some() {
                            w.lead_id = lead;
                        }
                        w.inbox_focus = false;
                        if product {
                            w.switch_view(CaseView::Product);
                        } else if let Some(action) = action {
                            w.plan_checked = vec![action];
                            w.plan_focus = true;
                        } else if let Some(gap) = gap {
                            w.switch_view(CaseView::Focus);
                            w.gap_focus = true;
                            w.gap_sel = w
                                .visible_gaps()
                                .iter()
                                .position(|g| g.id == gap)
                                .unwrap_or(0);
                        }
                    }
                    self.sync_workbench_plan();
                }
            }
            KeyCode::Char('/') => {
                self.focus = Focus::Prompt;
                return self.on_prompt_key(key);
            }
            _ => {}
        }
        false
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
            return format!("TNA · {}", report.title);
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
                CasePage::Network | CasePage::Investigation => None,
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
                CasePage::Closed | CasePage::Network | CasePage::Investigation => None,
            },
            Some(ModuleId::System) => match self.system_page {
                SystemPage::Settings => Some(ModuleId::Settings),
                SystemPage::Hardware => Some(ModuleId::Hardware),
                SystemPage::Log => None,
            },
            Some(ModuleId::Providers) => match self.provider_page {
                ProviderPage::Osint => Some(ModuleId::Osint),
                _ => Some(ModuleId::Providers),
            },
            Some(ModuleId::Settings) => Some(ModuleId::Settings),
            Some(ModuleId::Hardware) => Some(ModuleId::Hardware),
            _ => None,
        }
    }

    pub fn turn_spinner(&self) -> &'static str {
        ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"][self.spinner % 10]
    }

    pub fn mode_label(&self) -> String {
        if self.module == Some(ModuleId::Providers) && self.chat_report.is_none() {
            return format!(
                "Providers · {} · Writer {}",
                self.provider_page.title(),
                self.active_model()
            );
        }
        let run = if self.running {
            format!(" {} {}", self.turn_spinner(), self.status)
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
        self.role_secret(true).model
    }

    pub fn role_secret(&self, writer: bool) -> ProviderSecret {
        provider::role_secret(&self.auth, &self.settings, writer)
    }

    pub fn role_label(&self, writer: bool) -> String {
        let secret = self.role_secret(writer);
        format!(
            "{} / {}",
            provider_name(&provider::effective_kind(&secret)),
            if secret.model.is_empty() {
                "Codex default"
            } else {
                &secret.model
            }
        )
    }

    pub fn picker_secret(&self) -> ProviderSecret {
        match self.model_target {
            ModelTarget::Writer => self.role_secret(true),
            ModelTarget::Tool => self.role_secret(false),
            ModelTarget::Connection => self.text_secret(),
        }
    }

    pub fn account_status(&self, kind: &str) -> String {
        if kind == "grok" {
            return self.grok_subscription_status.clone();
        }
        if let Some(status) = self.provider_checks.get(kind) {
            return status.clone();
        }
        let secret = provider::account_secret(&self.auth, kind);
        if secret
            .api_key
            .as_ref()
            .is_some_and(|key| !key.trim().is_empty())
        {
            "Key saved · not verified".into()
        } else if provider::resolved_key(&secret).is_some() {
            format!(
                "Using {} · not verified",
                provider::preset(kind)
                    .and_then(|p| p.env_key)
                    .unwrap_or("environment")
            )
        } else {
            "Not connected".into()
        }
    }

    pub fn model_choices(&self) -> Vec<(String, String)> {
        let secret = self.picker_secret();
        let kind = provider::effective_kind(&secret);
        let mut choices = Vec::new();
        if provider::effective_kind(&secret) == "grok" {
            for model in provider::grok_models() {
                choices.push((model.id.to_string(), model.name.to_string()));
            }
        }
        let remote = self
            .model_catalogs
            .get(&kind)
            .map(|catalog| catalog.iter().map(|m| m.id.clone()).collect::<Vec<_>>())
            .unwrap_or_else(|| {
                if kind == provider::effective_kind(&self.text_secret()) {
                    self.remote_models.clone()
                } else {
                    Vec::new()
                }
            });
        for id in &remote {
            if !choices.iter().any(|(existing, _)| existing == id) {
                choices.push((id.clone(), id.clone()));
            }
        }
        if !secret.model.is_empty() && !choices.iter().any(|(id, _)| id == &secret.model) {
            choices.push((secret.model.clone(), secret.model.clone()));
        }
        if kind == "openai-chatgpt" {
            choices.insert(
                0,
                (
                    "codex-default".into(),
                    "Codex default (subscription)".into(),
                ),
            );
        }
        choices
    }

    pub fn filtered_model_choices(&self) -> Vec<(String, String)> {
        let q = self.model_query.trim().to_lowercase();
        let mut choices: Vec<_> = self
            .model_choices()
            .into_iter()
            .filter(|(id, label)| {
                q.is_empty() || id.to_lowercase().contains(&q) || label.to_lowercase().contains(&q)
            })
            .collect();
        let query = self.model_query.trim();
        if choices.is_empty()
            && !query.is_empty()
            && query.len() <= 160
            && query
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "/:._-".contains(c))
        {
            choices.push((
                query.into(),
                format!("Use model ID: {query} (not verified)"),
            ));
        }
        choices
    }

    fn open_model_picker(&mut self) {
        self.model_target = ModelTarget::Writer;
        self.open_model_card();
    }

    fn open_model_card(&mut self) {
        self.model_picker = true;
        self.free_picker = false;
        self.model_query.clear();
        self.model_sel = 0;
        self.refresh_model_list();
    }

    fn request_catalog(&mut self, secret: ProviderSecret, draft: bool) {
        let kind = provider::effective_kind(&secret);
        if kind == "openai-chatgpt" || (kind == "grok" && self.grok_subscription_pending) {
            return;
        }
        let generation = self.catalog_generation.get(&kind).copied().unwrap_or(0) + 1;
        self.catalog_generation.insert(kind.clone(), generation);
        let checks = if draft {
            &mut self.provider_draft_checks
        } else {
            &mut self.provider_checks
        };
        checks.insert(kind.clone(), "Checking connection…".into());
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = provider::verified_catalog(&secret)
                .await
                .map_err(|err| err.to_string());
            let _ = tx.send(AppMsg::ModelList {
                kind,
                generation,
                draft,
                result,
            });
        });
    }

    fn refresh_model_list(&mut self) {
        let secret = self.picker_secret();
        if !self
            .model_catalogs
            .contains_key(&provider::effective_kind(&secret))
        {
            self.request_catalog(secret, false);
        }
    }

    pub fn filtered_free_models(&self) -> Vec<(String, String)> {
        let q = self.free_query.trim().to_lowercase();
        let secret = self.picker_secret();
        let kind = provider::effective_kind(&secret);
        let catalog = self.model_catalogs.get(&kind).unwrap_or(&self.catalog);
        provider::concrete_free_models(catalog)
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

    pub(crate) fn set_writer_model(&mut self, id: &str) {
        self.model_target = ModelTarget::Writer;
        self.select_model(id);
    }

    pub(crate) fn select_model(&mut self, id: &str) {
        if provider::is_free_router(id)
            && provider::effective_kind(&self.picker_secret()) == "openrouter"
        {
            self.open_free_picker();
            return;
        }
        let mut next = self.settings.clone();
        match self.model_target {
            ModelTarget::Writer => {
                next.writer_provider = provider::effective_kind(&self.picker_secret());
                next.writer_model = id.into();
            }
            ModelTarget::Tool => {
                next.tool_provider = provider::effective_kind(&self.picker_secret());
                next.tool_model = id.into();
            }
            ModelTarget::Connection => {
                next.model = id.into();
            }
        }
        if let Err(err) = self.save_settings_config(&next) {
            self.status = "Model save failed".into();
            self.log_event("system", &format!("model save failed: {err}"));
            return;
        }
        self.settings = next;
        if self.model_target == ModelTarget::Connection {
            let mut secret = self.text_secret();
            secret.model = id.into();
            self.auth.set_account(secret.clone());
            self.auth.text = Some(secret);
            if let Err(err) = self.save_auth_config(&self.auth) {
                self.log_event("system", &format!("legacy connection save failed: {err}"));
            }
        }
        self.status = format!("{} · {id}", self.picker_title());
        self.log_event("system", &self.status.clone());
        self.model_picker = false;
        self.free_picker = false;
        if self.form_module() == Some(ModuleId::Providers) {
            self.load_fields(ModuleId::Providers);
        }
    }

    pub fn picker_current(&self) -> String {
        match self.model_target {
            ModelTarget::Writer => self.role_secret(true).model,
            ModelTarget::Tool => self.role_secret(false).model,
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
            KeyCode::F(5) => self.request_catalog(self.picker_secret(), false),
            KeyCode::Up => {
                if count > 0 {
                    self.model_sel = (self.model_sel + count - 1) % count;
                }
            }
            KeyCode::Down => {
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
        if self.chat_report.is_some() {
            return &[];
        }
        self.transcripts
            .get(&self.session_id())
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    fn load_transcript(&mut self, id: &str) {
        if self.chat_report.is_some() && id.starts_with("report:") {
            return;
        }
        if self.transcripts.contains_key(id) {
            return;
        }
        let lines = self.store.load_messages(id).unwrap_or_default();
        self.transcripts.insert(id.to_string(), lines);
    }

    fn push_line(&mut self, role: &str, body: &str) {
        if self.chat_report.is_some() {
            self.status = body.into();
            return;
        }
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
            .tna_answer
            .as_ref()
            .map(|a| a.question.clone())
            .unwrap_or_default();
        let title = self
            .reports
            .iter()
            .find(|report| report.id == report_id)
            .map(|report| report.title.clone())
            .unwrap_or_else(|| "report".into());
        let secret = self.role_secret(true);
        let generation = self.turn_generation;
        // No configured provider: preserve the existing clipping fallback without a network call.
        let kind = provider::effective_kind(&secret);
        if kind != "openai-chatgpt"
            && (secret.base_url.trim().is_empty()
                || secret.model.trim().is_empty()
                || (provider::resolved_key(&secret).is_none()
                    && kind != "local"
                    && !(kind == "grok" && argos_osint_core::grok_oauth::auth_path().exists())
                    && secret.device.is_none()))
        {
            self.store_report_insight(report_id, brain::clip_fact(&answer), generation);
            return;
        }
        self.pending_tna_insights
            .insert(generation, (report_id.clone(), brain::clip_fact(&answer)));
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let fact = match distill_insight(&secret, &title, &question, &answer).await {
                Ok(text) if !text.trim().is_empty() => text,
                _ => brain::clip_fact(&answer),
            };
            let fact = brain::clip_fact(&fact);
            if fact.is_empty() {
                return;
            }
            let _ = tx.send(AppMsg::Insight {
                report_id,
                fact,
                generation,
            });
        });
    }

    /// Distillation interrupted by normal shutdown uses the existing clipped-fact fallback.
    fn persist_pending_insights(&mut self) {
        for (generation, (report_id, fact)) in std::mem::take(&mut self.pending_tna_insights) {
            self.store_report_insight(report_id, fact, generation);
        }
    }

    fn store_report_insight(&mut self, report_id: String, fact: String, generation: u64) {
        if fact.trim().is_empty() || !self.reports.iter().any(|r| r.id == report_id) {
            return;
        }
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
            if generation == self.turn_generation {
                if let Some(answer) = self.tna_answer.as_mut() {
                    answer.filed = true;
                }
            }
            return;
        }
        match self.store.add_report_fact(&text, &report_id) {
            Ok(memory) => {
                self.memories.insert(0, memory);
                self.log_event("system", "fact filed from report network");
                if self.chat_report.as_deref() == Some(report_id.as_str())
                    && generation == self.turn_generation
                {
                    if let Some(answer) = self.tna_answer.as_mut() {
                        answer.filed = true;
                    }
                }
            }
            Err(err) => {
                self.log_event("system", &format!("report insight save failed: {err}"));
                if self.chat_report.as_deref() == Some(report_id.as_str())
                    && generation == self.turn_generation
                {
                    self.status = "insight error".into();
                    if let Some(answer) = self.tna_answer.as_mut() {
                        answer.error = Some(format!(
                            "Answer completed, but its insight could not be filed: {err}"
                        ));
                    }
                }
            }
        }
    }

    fn replace_last_assistant(&mut self, body: &str) {
        if self.chat_report.is_some() {
            return;
        }
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
        if self.chat_report.is_some() {
            return;
        }
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
                "Evidence-only TNA workspace for report {} — {}. Answer only from the supplied passages in the explicit scope and its existing graph. No investigation, research tools, Brain memories or old chat history. Missing links indicate absent co-occurrence, not intelligence gaps. If the evidence is unavailable, say so.\n{}",
                report.id, report.title, self.tna_digest()
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
        let mut reports: Vec<_> = self
            .reports
            .iter()
            .filter(|r| match &self.evidence_scope {
                argos_osint_core::evidence::EvidenceScope::Desk => true,
                argos_osint_core::evidence::EvidenceScope::Case(id) => {
                    r.case_id.as_ref() == Some(id)
                }
                argos_osint_core::evidence::EvidenceScope::Report(id) => &r.id == id,
                argos_osint_core::evidence::EvidenceScope::Reports(ids) => ids.contains(&r.id),
                argos_osint_core::evidence::EvidenceScope::Collection => true,
            })
            .cloned()
            .collect();
        reports.sort_by_key(|r| {
            self.recommendations
                .iter()
                .position(|h| h.report_id == r.id)
                .unwrap_or(usize::MAX)
        });
        rows.extend(
            self.cases
                .iter()
                .filter(|c| {
                    !self.pending_reports.iter().any(|p| p.case_id == c.id)
                        && match &self.evidence_scope {
                            argos_osint_core::evidence::EvidenceScope::Desk
                            | argos_osint_core::evidence::EvidenceScope::Collection => true,
                            argos_osint_core::evidence::EvidenceScope::Case(id) => &c.id == id,
                            _ => false,
                        }
                })
                .cloned()
                .map(ReportRow::Case),
        );
        rows.extend(reports.into_iter().map(ReportRow::Completed));
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
                ReportRow::Case(case) => out.push_str(&format!(
                    "Case {}: {} · /case {}\n",
                    case.id, case.title, case.id
                )),
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
                "Providers / {}. Grok setup is subscription-only through Grok Build. OpenRouter has its own API account. OpenAI setup is ChatGPT subscription only, through Codex CLI (Writer only). No OpenAI API-key setup and no Mail/MCP configuration in this app. Models assigns Writer {} and Tools {} independently. Account setup never changes the other provider's credentials.",
                self.provider_page.title(), self.role_label(true), self.role_label(false)
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
                argos_osint_core::workers::spawn_blocking(move || hardware::profile_cached(fresh))
                    .await;
            if let Ok(profile) = profile {
                let _ = tx.send(AppMsg::Hardware(profile));
            }
        });
    }

    pub fn on_msg(&mut self, msg: AppMsg) {
        match msg {
            AppMsg::CaseDrafted {case_id,result} => {
                if let Some(count)=self.case_pending_work.get_mut(&case_id) {*count=count.saturating_sub(1);}
                self.on_msg(AppMsg::ReportUpdated(result));
            },
            AppMsg::CaseDataPlanned {generation,result} => {
                if generation == self.case_data_generation {
                    match result { Ok(plan) => self.show_case_data_plan(plan), Err(err) => self.status=err }
                }
            },
            AppMsg::CaseDataApplied {plan,result} => self.finish_case_data(plan,result),
            AppMsg::DeskCaseReady{case_id,generation,result}=>{
                if self.desk_generations.get(&case_id)==Some(&generation) && self.cases.iter().any(|c|c.id==case_id) {self.desk_refreshing.remove(&case_id);match result {Ok(data)=>{self.desk_cases.insert(case_id,data);},Err(e)=>self.status=e}}
            },
            AppMsg::CaseReady {case_id,generation,result} => {
                if generation==self.case_generation {
                    if let Some(w)=self.investigation.as_mut().filter(|w|w.case_id==case_id) {w.loading=false;match result {Ok(data)=>{self.desk_generations.entry(case_id.clone()).and_modify(|g|*g=g.wrapping_add(1)).or_insert(1);self.desk_refreshing.remove(&case_id);self.desk_cases.insert(case_id.clone(),data.clone());w.snapshot=data.snapshot.clone();if let Some(scope)=&data.scope{w.question=scope.question.clone();}w.data=data;if w.lead_id.as_ref().is_none_or(|id|!w.data.entities.iter().any(|e|&e.id==id)){w.lead_id=w.data.entities.first().map(|e|e.id.clone());}},Err(e)=>self.status=e}}
                }
            },
            AppMsg::CaseSource {case_id,generation,result} => {
                if generation==self.case_source_generation {if let Some(w)=self.investigation.as_mut().filter(|w|w.case_id==case_id){w.source=Some(result.unwrap_or_else(|e|e));w.scroll=0;}}
            },
            AppMsg::ReviewedRetrieved {generation,query,result} => {
                if generation==self.turn_generation {let passages=result.map(|(p,f)|{self.reviewed_recommendations=f;p});self.answer_retrieval(generation,query,passages);}
            },
            AppMsg::Coverage {report_id,result} => {if report_id==self.chat_report {match result {Ok(rows)=>self.coverage_rows=rows,Err(err)=>self.status=err}}},
            AppMsg::OpenPassage(result) => match result {Ok(Some(hit))=>{let citation=hit.citation();self.recommendations.push(hit);self.run_slash(&format!("/cite {citation}"));},Ok(None)=>self.status="No supporting passage in this cell; unavailable evidence is not a zero finding".into(),Err(err)=>self.status=err},
            AppMsg::ToolManaged {name,result} => {
                self.research_active=self.research_active.saturating_sub(1);
                match result {
                    Ok(path)=>{if let Some(c)=self.settings.research.get_mut(&name){c.executable=path.unwrap_or_default();c.readiness=if c.executable.is_empty(){argos_osint_core::research::Readiness::NotInstalled}else{argos_osint_core::research::Readiness::Degraded};}let _=self.save_settings_config(&self.settings);self.status="Managed tool action completed; collection readiness shown separately".into();},
                    Err(err)=>{if let Some(c)=self.settings.research.get_mut(&name){c.readiness=if c.executable.is_empty(){argos_osint_core::research::Readiness::Failed}else{argos_osint_core::research::Readiness::Degraded};}self.status=format!("Tool action failed: {err}");self.log_event("system",&self.status.clone());}
                }
                if self.provider_page==ProviderPage::Research {self.load_research_fields();}
            },
            AppMsg::ResearchProgress(job)=>{if !self.store.research_job_is_current(&job).unwrap_or(false){return;}if let Some(w)=self.investigation.as_mut().filter(|w|job.input.case_id.as_ref()==Some(&w.case_id)){w.data.jobs.retain(|j|j.id!=job.id);w.data.jobs.insert(0,job.clone());}if let Some(data)=job.input.case_id.as_ref().and_then(|id|self.desk_cases.get_mut(id)){data.jobs.retain(|j|j.id!=job.id);data.jobs.insert(0,job.clone());}self.research_jobs.retain(|j|j.id!=job.id);self.research_jobs.insert(0,job);self.research_jobs.truncate(200);},
            AppMsg::ResearchJob(job) => {
                let affected=self.investigation.as_ref().is_some_and(|w|job.input.case_id.as_ref()==Some(&w.case_id));
                self.research_active=self.research_active.saturating_sub(1);
                if let Some(count)=job.input.case_id.as_ref().and_then(|id|self.case_pending_work.get_mut(id)){*count=count.saturating_sub(1);}
                if !self.store.research_job_is_current(&job).unwrap_or(false){return;}
                self.status=format!("{}: {:?} · {}",job.provider,job.state,job.progress);
                self.log_event("task",&format!("{} {:?}: {}",job.provider,job.state,job.error.as_deref().unwrap_or(&job.progress)));
                self.research_jobs.retain(|j|j.id!=job.id);self.research_jobs.insert(0,job);
                self.research_jobs.truncate(200);
                if let Some(id)=self.research_jobs.first().and_then(|j|j.input.case_id.clone()){self.refresh_desk_case(&id);}
                if affected{self.refresh_investigation();}
            },
            AppMsg::ResearchTest {name,state,detail} => {
                if let Some(c)=self.settings.research.get_mut(&name){if detail.starts_with("Version matched"){c.detected_version=c.supported_version.clone();}c.readiness=state;}
                self.status=detail.clone();self.log_event("system",&format!("{name}: {detail}"));
                if self.provider_page==ProviderPage::Research {self.load_research_fields();}
            },
            AppMsg::EvidenceSurface {report_id,title,result,scope} => {
                if scope!=self.evidence_scope{return;}
                if let argos_osint_core::evidence::EvidenceScope::Case(id)=&scope {let id=id.clone();self.refresh_desk_case(&id);}
                self.refresh_investigation();
                if report_id==self.chat_report {
                    let text=result.unwrap_or_else(|e|format!("Evidence unavailable: {e}"));
                    if self.chat_report.is_some() {self.tna_answer=Some(TnaAnswer {question:title,answer:text,..Default::default()});self.tna_answer_scroll=0;}
                    else if let Some(w)=self.investigation.as_mut(){w.source=Some(format!("{title}\n\n{text}"));w.scroll=0;}else {self.push_line("assistant",&format!("{title}\n\n{text}"));}
                }
            },
            AppMsg::ReportUpdated(result) => match result {
                Ok(report)=>{let id=report.id.clone();let _=self.reload_lists();self.recommendations.clear();self.after_report_filed(&id);self.status="Report update saved; earlier citations retained".into();if self.chat_report.as_deref()==Some(id.as_str()){self.open_report_chat(&id);}},
                Err(err)=>self.status=format!("Report update failed: {err}"),
            },
            AppMsg::ReportSource {report_id,generation,version,result} => {
                if self.chat_report.as_ref()==Some(&report_id) && generation==self.report_source_generation {
                    match result {Ok(text)=>{if version.is_some(){self.report_read_source=Some(text);}else{self.tna_source=text;}self.tna_source_error=None;self.focus_selected_passage();},Err(err)=>self.tna_source_error=Some(err)}
                }
            },
            #[cfg(test)]
            AppMsg::Turn(ev) => self.on_turn(ev),
            AppMsg::ScopedTurn {
                generation,
                report_id,
                event,
            } => {
                if generation == self.turn_generation && report_id == self.chat_report {
                    self.on_turn(event);
                }
            }
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
            AppMsg::ModelList {
                kind,
                generation,
                draft,
                result,
            } => {
                if self.catalog_generation.get(&kind).copied() != Some(generation) {
                    return;
                }
                match result {
                    Ok(models) => {
                        let checks = if draft {
                            &mut self.provider_draft_checks
                        } else {
                            &mut self.provider_checks
                        };
                        checks.insert(
                            kind.clone(),
                            format!(
                                "{} · {} models",
                                if draft {
                                    "Draft verified · Save to use"
                                } else {
                                    "Connected"
                                },
                                models.len()
                            ),
                        );
                        if !draft && provider::effective_kind(&self.text_secret()) == kind {
                            self.remote_models = models.iter().map(|m| m.id.clone()).collect();
                            self.catalog = models.clone();
                        }
                        if !draft {
                            self.model_catalogs.insert(kind.clone(), models);
                        }
                        self.status = format!("{} connected", provider_name(&kind));
                        self.log_event(
                            "api",
                            &format!("{} connection verified", provider_name(&kind)),
                        );
                    }
                    Err(err) => {
                        let checks = if draft {
                            &mut self.provider_draft_checks
                        } else {
                            &mut self.provider_checks
                        };
                        checks.insert(kind.clone(), "Connection failed · see System log".into());
                        self.status = format!("{} connection failed", provider_name(&kind));
                        self.log_event(
                            "api",
                            &format!("{} connection failed: {err}", provider_name(&kind)),
                        );
                    }
                }
                if kind == "grok" {
                    self.grok_subscription_status = self
                        .provider_checks
                        .get("grok")
                        .cloned()
                        .unwrap_or_else(|| "Grok subscription not checked".into());
                }
                self.free_loading = false;
            }
            AppMsg::GrokSubscriptionProgress { generation, line } => {
                if self.catalog_generation.get("grok").copied() != Some(generation) {
                    return;
                }
                if self.grok_subscription_instructions.len() == 8 {
                    self.grok_subscription_instructions.remove(0);
                }
                self.grok_subscription_instructions.push(line);
                self.grok_subscription_status = "Grok sign-in in progress…".into();
            }
            AppMsg::GrokSubscriptionCheck { generation, result } => {
                if self.catalog_generation.get("grok").copied() != Some(generation) {
                    return;
                }
                self.grok_subscription_pending = false;
                self.grok_subscription_instructions.clear();
                match result {
                    Ok(models) => {
                        self.grok_subscription_status =
                            format!("Grok subscription connected · {} models", models.len());
                        self.provider_checks
                            .insert("grok".into(), self.grok_subscription_status.clone());
                        if provider::effective_kind(&self.text_secret()) == "grok" {
                            self.remote_models = models.iter().map(|m| m.id.clone()).collect();
                            self.catalog = models.clone();
                        }
                        self.model_catalogs.insert("grok".into(), models);
                    }
                    Err(err) => {
                        self.model_catalogs.remove("grok");
                        if provider::effective_kind(&self.text_secret()) == "grok" {
                            self.remote_models.clear();
                            self.catalog.clear();
                        }
                        self.grok_subscription_status = err.clone();
                        self.provider_checks.insert(
                            "grok".into(),
                            "Grok subscription verification failed".into(),
                        );
                        self.log_event("api", &format!("Grok subscription: {err}"));
                    }
                }
            }
            AppMsg::SubscriptionProgress(line) => {
                if self.subscription_instructions.len() == 8 {
                    self.subscription_instructions.remove(0);
                }
                self.subscription_instructions.push(line);
                self.subscription_status = "Waiting for ChatGPT sign-in…".into();
            }
            AppMsg::SubscriptionCheck(result) => {
                self.subscription_pending = false;
                self.subscription_instructions.clear();
                self.subscription_status = match result {
                    Ok(status) => status,
                    Err(err) => {
                        self.log_event("api", &err);
                        err
                    }
                };
            }
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
            #[cfg(test)]
            AppMsg::Research { case_id, result } => self.finish_research(case_id, result),
            AppMsg::Insight {
                report_id,
                fact,
                generation,
            } => {
                self.pending_tna_insights.remove(&generation);
                self.store_report_insight(report_id, fact, generation);
            }
            AppMsg::TnaReady {
                targeted,
                report_id,
                scope,
                result,
            } => {if !targeted || scope.as_ref()==Some(&self.evidence_scope) {self.on_tna_ready(targeted, report_id, result);}},
        }
    }

    fn on_turn(&mut self, ev: TurnEvent) {
        if self.chat_report.is_some() {
            self.on_tna_turn(ev);
            return;
        }
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
                let _ = self.store.add_report_metadata(&meta);
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
                // Discussion remains ephemeral. Saving knowledge is an explicit analyst action.
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
                        let _ = self.store.add_report_metadata(&meta);
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
            Event::Paste(text) if self.editing => {
                if let Some(kind) = self
                    .provider_page
                    .account()
                    .filter(|_| self.module == Some(ModuleId::Providers))
                {
                    self.provider_draft_checks.remove(kind);
                    self.catalog_generation
                        .entry(kind.into())
                        .and_modify(|g| *g += 1)
                        .or_insert(1);
                }
                if let Some(field) = self.fields.get_mut(self.field_sel) {
                    field.value.push_str(&text.replace(['\r', '\n'], ""));
                }
                false
            }
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
        if self.provider_picker.is_some()
            || self.free_picker
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
        if self.module == Some(ModuleId::Cases) && self.case_page == CasePage::Investigation {
            if let Some(view) = self.investigation.as_ref().and_then(|w| {
                w.tab_hits
                    .iter()
                    .find(|(_, rect)| rect.contains(pos))
                    .map(|(view, _)| *view)
            }) {
                self.select_investigation_view(view);
                return;
            }
        }
        if self.report_area.contains(pos) {
            self.focus = Focus::Reports;
            let rel = y.saturating_sub(self.report_area.y + 1) as usize;
            if let Some(Some(index)) = self.report_line_index.get(rel).copied() {
                self.report_sel = index;
                self.ask_open_report();
            }
            return;
        }
        if self.chat_report.is_some() {
            if let Some(index) = self.tna_tab_hits.iter().position(|tab| tab.contains(pos)) {
                self.set_tna_layout(TnaLayout::ALL[index]);
                return;
            }
        }
        if let Some(index) = self.case_tab_hits.iter().position(|tab| tab.contains(pos)) {
            if let Some(page) = self.case_pages().get(index).copied() {
                self.select_case_page(page);
            }
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
        if self.module == Some(ModuleId::Providers) {
            if let Some((index, _)) = self
                .provider_field_hits
                .iter()
                .find(|(_, area)| area.contains(pos))
            {
                self.field_sel = *index;
                self.focus = Focus::Canvas;
                self.activate_field();
                return;
            }
        }
        if let Some(index) = self
            .system_tab_hits
            .iter()
            .position(|tab| tab.contains(pos))
        {
            self.select_system_page(SystemPage::all()[index]);
            return;
        }
        if self.case_page == CasePage::Network {
            if self.tna_table_detail_area.contains(pos) {
                self.focus = Focus::TableDetail;
                return;
            }
            if self.tna_table_list_area.contains(pos) {
                self.focus = Focus::Graph;
                if y >= self.tna_table_list_area.y + 2 {
                    let row = (y - self.tna_table_list_area.y - 2) as usize
                        + self.tna_table_state.offset();
                    if row < self.tna_selectable_count() {
                        self.tna_sel = row;
                        self.tna_detail_scroll = 0;
                        self.pin_tna_selection(self.tna_selected_item(), true);
                        self.sync_tna_table_ui();
                    }
                }
                return;
            }
        }
        if self.canvas_area.contains(pos) {
            self.focus = if self.chat_report.is_some() {
                Focus::Graph
            } else {
                Focus::Canvas
            };
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

    pub fn case_pages(&self) -> Vec<CasePage> {
        if self.chat_report.is_some() || self.investigation.is_some() {
            Vec::new()
        } else {
            CasePage::all().to_vec()
        }
    }

    fn select_case_page(&mut self, page: CasePage) {
        if self.chat_report.is_some() && page != CasePage::Network {
            self.status = "Esc closes the report workspace".into();
            return;
        }
        if page == CasePage::Network && self.open_report().is_none() {
            self.status = "open a filed report to view its network".into();
            return;
        }
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
            self.tna_focus_id = None;
            self.tna_detail_scroll = 0;
            self.clamp_tna_sel();
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
        if self.provider_picker.is_some() && !(ctrl && key.code == KeyCode::Char('c')) {
            return self.on_provider_picker_key(key);
        }
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
        if self.on_case_desk()
            && self.chat_report.is_none()
            && self.investigation.is_none()
            && !self.editing
        {
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
        if self.investigation.is_some() && self.on_case_desk() {
            return self.on_investigation_key(key);
        }
        if self.module == Some(ModuleId::Providers)
            && self.provider_page == ProviderPage::Research
            && !self.editing
            && self.focus == Focus::Canvas
        {
            if let KeyCode::Char(c @ '1'..='5') = key.code {
                self.research_phase =
                    argos_osint_core::research::ResearchPhase::ALL[(c as u8 - b'1') as usize];
                self.research_sel = 0;
                self.field_sel = 0;
                self.load_research_fields();
                return false;
            }
        }
        if self.chat_report.is_some() && self.on_case_desk() {
            if key.code == KeyCode::Esc {
                self.on_esc();
                return false;
            }
            if key.code == KeyCode::Tab {
                self.tna_find_editing = false;
                self.focus = self.next_focus();
                return false;
            }
            if self.tna_find_editing {
                return self.on_tna_key(key);
            }
            if self.focus == Focus::Prompt || !self.prompt.is_empty() {
                self.focus = Focus::Prompt;
                return self.on_prompt_key(key);
            }
            if self.focus != Focus::Launcher {
                return self.on_tna_key(key);
            }
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
        if self.on_case_desk()
            && self.investigation.is_none()
            && self.chat_report.is_none()
            && self.case_page == CasePage::Closed
        {
            if key.code == KeyCode::Char('\\') {
                self.desk_transcript = !self.desk_transcript;
                return false;
            }
            if self.focus != Focus::Prompt
                && matches!(
                    key.code,
                    KeyCode::Char('j' | 'k' | 'g' | 'q' | 'w')
                        | KeyCode::Up
                        | KeyCode::Down
                        | KeyCode::Enter
                )
            {
                return self.on_operations_desk_key(key);
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
            _ if self.focus == Focus::Graph || self.focus == Focus::TableDetail => {
                self.on_tna_key(key)
            }
            _ => self.on_canvas_key(key),
        }
    }

    fn cancel_or_quit(&mut self) -> bool {
        if self.running {
            self.cancel.store(true, Ordering::Relaxed);
            self.status = "cancelling".into();
            if self.chat_report.is_some() || self.investigation.is_some() {
                self.turn_generation = self.turn_generation.wrapping_add(1);
                self.running = false;
                if let Some(answer) = self.tna_answer.as_mut() {
                    answer.pending = false;
                    answer.error = Some("Cancelled; no insight filed.".into());
                }
            }
            false
        } else if !self.prompt.is_empty() && self.focus == Focus::Prompt {
            self.prompt.clear();
            self.cursor = 0;
            false
        } else if self.research_active > 0 {
            self.status = "Background jobs continue · /cancel-jobs cancels them".into();
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
        if self.investigation.is_some()
            && self
                .canvas_area
                .contains(ratatui::layout::Position { x, y })
        {
            if let Some(w) = self.investigation.as_mut().filter(|w| w.source.is_some()) {
                w.scroll = (w.scroll as isize + delta).max(0) as usize;
            } else {
                let code = if delta > 0 {
                    KeyCode::Up
                } else {
                    KeyCode::Down
                };
                self.on_investigation_key(KeyEvent::new(code, KeyModifiers::NONE));
            }
            return;
        }
        let pos = ratatui::layout::Position { x, y };
        if self.on_case_desk() && self.case_page == CasePage::Brain {
            // ScrollUp (delta>0) → previous row; ScrollDown → next (match Table).
            if self.brain_list_area == Rect::default() || self.brain_list_area.contains(pos) {
                let steps = delta.unsigned_abs().max(1);
                let dir = if delta > 0 { -1 } else { 1 };
                for _ in 0..steps {
                    self.move_brain_sel(dir);
                }
            }
            return;
        }
        if self.on_case_desk() && self.case_page == CasePage::Network {
            self.scroll_tna_table_at(pos, delta);
            return;
        }
        if self.focus == Focus::Canvas || self.canvas_area.contains(pos) {
            self.scroll_chat(delta);
        }
    }

    /// Wheel on Table master–detail: only the focused pane moves.
    fn scroll_tna_table_at(&mut self, _pos: ratatui::layout::Position, delta: isize) {
        let steps = delta.unsigned_abs().max(1);
        match self.focus {
            Focus::TableDetail => {
                if delta > 0 {
                    self.tna_detail_scroll = self.tna_detail_scroll.saturating_sub(steps);
                } else {
                    self.tna_detail_scroll = self.tna_detail_scroll.saturating_add(steps);
                }
                self.sync_tna_detail_scroll_state();
            }
            Focus::Graph => {
                // ScrollUp (delta>0) → previous row; ScrollDown → next.
                let dir = if delta > 0 { -1 } else { 1 };
                for _ in 0..steps {
                    self.tna_step_sel(dir);
                }
            }
            _ => {}
        }
    }

    fn next_focus(&self) -> Focus {
        if self.investigation.is_some() {
            return match self.focus {
                Focus::Prompt => Focus::Graph,
                _ => Focus::Prompt,
            };
        }
        let reports = self.on_case_desk() && self.case_page == CasePage::Closed;
        let network = self.on_case_desk() && self.case_page == CasePage::Network;
        let table = network;
        match self.focus {
            Focus::Launcher => {
                if network {
                    Focus::Graph
                } else {
                    Focus::Canvas
                }
            }
            Focus::Graph if table => Focus::TableDetail,
            Focus::Graph => Focus::TableDetail,
            Focus::TableDetail => Focus::Prompt,
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
            ReportRow::Case(case) => self.open_investigation(&case.id),
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
            0 => {
                if let Some(hit) = self
                    .recommendations
                    .iter()
                    .find(|h| h.report_id == id)
                    .cloned()
                {
                    self.run_slash(&format!("/cite {}", hit.citation()));
                } else {
                    self.open_report_chat(&id);
                }
            }
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
            self.close_report_workspace();
        }
        self.after_report_deleted(id);
        let _ = self.reload_lists();
        self.status = "report deleted".into();
    }

    pub fn tna_snapshot(&self) -> Option<&TnaSnapshot> {
        if let Some(w) = &self.investigation {
            return w.snapshot.as_ref();
        }
        let report_id = self.chat_report.as_deref()?;
        self.tna_report.as_ref().filter(|snap| match &snap.scope {
            argos_osint_core::tna::TnaScope::Targeted { report_id: id, .. } => id == report_id,
            argos_osint_core::tna::TnaScope::Selected { report_ids, .. } => {
                report_ids.iter().any(|id| id == report_id)
            }
            _ => false,
        })
    }

    /// Find-filter over the report network table.
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

    /// All entities within five hops, nearest first. The renderer pages these
    /// candidates to fit the terminal without dropping deeper connections.
    pub fn tna_display_nodes(&self) -> Vec<TnaDisplayItem> {
        let Some(snap) = self.tna_snapshot() else {
            return Vec::new();
        };
        let Some(focus) = self.tna_focus_id.as_deref().or_else(|| {
            self.tna_visible_nodes()
                .get(self.tna_sel)
                .map(|i| snap.nodes[*i].id.as_str())
        }) else {
            return Vec::new();
        };
        let adj = Self::tna_adjacency(snap);
        let mut distances = HashMap::from([(focus.to_string(), 0u32)]);
        let mut queue = std::collections::VecDeque::from([focus.to_string()]);
        while let Some(id) = queue.pop_front() {
            let distance = distances[&id];
            if distance >= TNA_DETAIL_HOPS {
                continue;
            }
            for neighbor in adj.get(&id).into_iter().flatten() {
                if !distances.contains_key(neighbor) {
                    distances.insert(neighbor.clone(), distance + 1);
                    queue.push_back(neighbor.clone());
                }
            }
        }
        let mut indices: Vec<_> = snap
            .nodes
            .iter()
            .enumerate()
            .filter(|(_, n)| distances.contains_key(&n.id))
            .map(|(i, _)| i)
            .collect();
        indices.sort_by(|&a, &b| {
            distances[&snap.nodes[a].id]
                .cmp(&distances[&snap.nodes[b].id])
                .then(snap.nodes[a].id.cmp(&snap.nodes[b].id))
        });
        indices
            .into_iter()
            .map(|idx| TnaDisplayItem::Real { idx })
            .collect()
    }

    /// Keep focus visible and browse remaining entities with detail j/k/wheel.
    pub fn tna_detail_box_items(&self, fit: usize) -> Vec<TnaDisplayItem> {
        let items = self.tna_display_nodes();
        let fit = fit.min(TNA_GRAPH_BOX_BUDGET);
        if fit == 0 || items.is_empty() {
            return Vec::new();
        }
        if fit == 1 && self.tna_detail_scroll > 0 {
            return items
                .into_iter()
                .skip(self.tna_detail_scroll)
                .take(1)
                .collect();
        }
        let offset = self.tna_detail_scroll.min(items.len().saturating_sub(2));
        let mut shown = vec![items[0].clone()];
        shown.extend(items.iter().skip(1 + offset).take(fit - 1).cloned());
        if fit > 1 && offset == 0 {
            if let (Some(neighbor), Some(snap)) = (self.tna_ledger_neighbor(), self.tna_snapshot())
            {
                if let Some(item) = items
                    .iter()
                    .find(|TnaDisplayItem::Real { idx }| snap.nodes[*idx].id == neighbor)
                {
                    if let Some(index) = shown
                        .iter()
                        .position(|TnaDisplayItem::Real { idx }| snap.nodes[*idx].id == neighbor)
                    {
                        shown.swap(1, index);
                    } else if shown.len() > 1 {
                        shown[1] = item.clone();
                    }
                }
            }
        }
        shown
    }

    pub fn tna_selection_list(&self) -> Vec<TnaDisplayItem> {
        self.tna_visible_nodes()
            .into_iter()
            .map(|idx| TnaDisplayItem::Real { idx })
            .collect()
    }

    pub fn tna_selected_item(&self) -> Option<TnaDisplayItem> {
        self.tna_selection_list().get(self.tna_sel).cloned()
    }

    /// Re-pin selection index in the display list. When `move_focus` is set,
    /// the selected node becomes the egocentric center (rebuilds the 16-box view).
    fn pin_tna_selection(&mut self, item: Option<TnaDisplayItem>, move_focus: bool) {
        let Some(item) = item else {
            return;
        };
        if move_focus {
            match &item {
                TnaDisplayItem::Real { idx } => {
                    if let Some(snap) = self.tna_snapshot() {
                        if let Some(n) = snap.nodes.get(*idx) {
                            self.tna_focus_id = Some(n.id.clone());
                        }
                    }
                }
            }
        }
    }

    pub fn tna_selectable_count(&self) -> usize {
        self.tna_visible_nodes().len()
    }

    fn clamp_tna_sel(&mut self) {
        let n = self.tna_selectable_count();
        if n == 0 {
            self.tna_sel = 0;
        } else if self.tna_sel >= n {
            self.tna_sel = n - 1;
        }
        if self.tna_focus_id.as_ref().is_none_or(|id| {
            !self
                .tna_visible_nodes()
                .iter()
                .any(|i| self.tna_snapshot().is_some_and(|s| s.nodes[*i].id == *id))
        }) {
            self.tna_focus_id = None;
            if let Some(TnaDisplayItem::Real { idx }) = self.tna_selected_item() {
                if let Some(snap) = self.tna_snapshot() {
                    if let Some(n) = snap.nodes.get(idx) {
                        self.tna_focus_id = Some(n.id.clone());
                    }
                }
            }
        }
        self.sync_tna_table_ui();
    }

    const TNA_TABLE_ITEM_HEIGHT: usize = 1;

    /// Keep `TableState` / list scrollbar aligned with `tna_sel`.
    pub fn sync_tna_table_ui(&mut self) {
        let n = self.tna_visible_nodes().len();
        if n == 0 {
            self.tna_table_state.select(None);
            self.tna_table_scroll = ScrollbarState::new(0).position(0);
        } else {
            let i = self.tna_sel.min(n - 1);
            self.tna_table_state.select(Some(i));
            let content = n.saturating_sub(1) * Self::TNA_TABLE_ITEM_HEIGHT;
            self.tna_table_scroll =
                ScrollbarState::new(content).position(i * Self::TNA_TABLE_ITEM_HEIGHT);
        }
        self.sync_tna_detail_scroll_state();
    }

    pub fn sync_tna_detail_scroll_state(&mut self) {
        let max_pos = self.tna_display_nodes().len().saturating_sub(1);
        self.tna_detail_scroll = self.tna_detail_scroll.min(max_pos);
    }

    fn ensure_tna_snapshot(&mut self, force: bool) {
        let Some(id) = self.open_report().map(|report| report.id.clone()) else {
            return;
        };
        if !force {
            if self.tna_snapshot().is_some() {
                return;
            }
            if self.tna_db_path.is_none() {
                if let Ok(Some(snap)) = self.store.get_tna_graph(&report_key(&id)) {
                    self.tna_report = Some(snap);
                    self.clamp_tna_sel();
                    return;
                }
            }
        }
        if force || self.tna_pending_report.as_deref() != Some(id.as_str()) {
            self.spawn_tna_rebuild(true);
        }
    }

    fn spawn_tna_rebuild(&mut self, targeted: bool) {
        if targeted && self.chat_report.is_none() {
            return;
        }
        if targeted {
            self.tna_pending_report = self.chat_report.clone();
        }
        if let Some(path) = self.tna_db_path.clone() {
            let evidence_scope = self.evidence_scope.clone();
            let event_scope = evidence_scope.clone();
            self.tna_rebuilding = true;
            self.tna_rebuild_gen = self.tna_rebuild_gen.saturating_add(1);
            let report_id = if targeted {
                self.chat_report.clone()
            } else {
                None
            };
            let tx = self.tx.clone();
            argos_osint_core::workers::spawn_blocking(move || {
                let opened = Store::open(&path).map_err(|err| err.to_string());
                let result = opened.and_then(|store| {
                    if let Some(id) = report_id.as_deref() {
                        store.sync_report_index().map_err(|err| err.to_string())?;
                        if matches!(
                            evidence_scope,
                            argos_osint_core::evidence::EvidenceScope::Case(_)
                                | argos_osint_core::evidence::EvidenceScope::Reports(_)
                                | argos_osint_core::evidence::EvidenceScope::Collection
                        ) {
                            let corpus = tna::TnaCorpus::scoped(&store, &evidence_scope)
                                .map_err(|err| err.to_string())?;
                            let corrections = corpus
                                .docs
                                .iter()
                                .filter_map(|d| store.corrections(&d.report_id).ok())
                                .flatten()
                                .collect::<Vec<_>>();
                            let mut snapshot = tna::build_snapshot_corrected(&corpus, &corrections);
                            let decisions = corpus
                                .docs
                                .iter()
                                .filter_map(|d| store.identity_decisions(&d.report_id).ok())
                                .flatten()
                                .collect::<Vec<_>>();
                            tna::apply_identity_decisions(&mut snapshot, &decisions);
                            Ok(snapshot)
                        } else if let Ok(Some(snapshot)) = store.get_tna_graph(&report_key(id)) {
                            Ok(snapshot)
                        } else {
                            tna::rebuild_for_report(&store, id).map_err(|err| err.to_string())
                        }
                    } else {
                        tna::rebuild_collection(&store).map_err(|err| err.to_string())
                    }
                });
                let _ = tx.send(AppMsg::TnaReady {
                    targeted,
                    report_id,
                    scope: Some(event_scope),
                    result,
                });
            });
            return;
        }
        self.rebuild_tna_sync(targeted);
    }

    fn rebuild_tna_sync(&mut self, targeted: bool) {
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
        self.tna_pending_report = None;
        self.clamp_tna_sel();
    }

    fn on_tna_ready(
        &mut self,
        targeted: bool,
        report_id: Option<String>,
        result: Result<TnaSnapshot, String>,
    ) {
        if targeted && report_id == self.tna_pending_report {
            self.tna_pending_report = None;
        }
        self.tna_rebuilding = self.tna_pending_report.is_some();
        match result {
            Ok(snap) => {
                if targeted {
                    if report_id.as_deref() == self.chat_report.as_deref() {
                        self.tna_report = Some(snap);
                        self.focus_selected_passage();
                    }
                } else {
                    self.tna_desk = Some(snap);
                }
            }
            Err(err) => self.log_event("task", &format!("tna rebuild failed: {err}")),
        }
        self.clamp_tna_sel();
    }

    fn after_report_filed(&mut self, report_id: &str) {
        if let Some(path) = self.tna_db_path.clone() {
            self.tna_rebuilding = true;
            let id = report_id.to_string();
            let tx = self.tx.clone();
            argos_osint_core::workers::spawn_blocking(move || {
                let opened = Store::open(&path).map_err(|err| err.to_string());
                match opened.and_then(|store| {
                    store.sync_report_index().map_err(|err| err.to_string())?;
                    tna::rebuild_after_file(&store, &id).map_err(|err| err.to_string())?;
                    let collection = store
                        .get_tna_graph(desk_key())
                        .map_err(|err| err.to_string())?;
                    let targeted = store
                        .get_tna_graph(&report_key(&id))
                        .map_err(|err| err.to_string())?;
                    Ok((collection, targeted))
                }) {
                    Ok((collection, targeted)) => {
                        if let Some(snap) = collection {
                            let _ = tx.send(AppMsg::TnaReady {
                                targeted: false,
                                report_id: None,
                                scope: None,
                                result: Ok(snap),
                            });
                        }
                        if let Some(snap) = targeted {
                            let _ = tx.send(AppMsg::TnaReady {
                                targeted: true,
                                scope: Some(argos_osint_core::evidence::EvidenceScope::Report(
                                    id.clone(),
                                )),
                                report_id: Some(id),
                                result: Ok(snap),
                            });
                        }
                    }
                    Err(err) => {
                        let _ = tx.send(AppMsg::TnaReady {
                            targeted: false,
                            report_id: None,
                            scope: None,
                            result: Err(err),
                        });
                    }
                }
            });
            return;
        }
        if let Err(err) = tna::rebuild_after_file(&self.store, report_id) {
            self.log_event("task", &format!("tna rebuild failed: {err}"));
            return;
        }
        if let Ok(Some(snap)) = self.store.get_tna_graph(desk_key()) {
            self.tna_desk = Some(snap);
        }
        if let Ok(Some(snap)) = self.store.get_tna_graph(&report_key(report_id)) {
            if self.chat_report.as_deref() == Some(report_id) {
                self.tna_report = Some(snap);
            }
        }
        if self.case_page == CasePage::Network {
            self.clamp_tna_sel();
        }
    }

    fn after_report_deleted(&mut self, report_id: &str) {
        if let Some(path) = self.tna_db_path.clone() {
            self.tna_report = None;
            self.tna_rebuilding = true;
            let id = report_id.to_string();
            let tx = self.tx.clone();
            argos_osint_core::workers::spawn_blocking(move || {
                let result = Store::open(&path)
                    .and_then(|store| {
                        tna::rebuild_after_delete(&store, &id)?;
                        let snap = store.get_tna_graph(desk_key())?.unwrap_or_else(|| {
                            TnaSnapshot::empty(argos_osint_core::tna::TnaScope::Collection)
                        });
                        Ok(snap)
                    })
                    .map_err(|err| err.to_string());
                let _ = tx.send(AppMsg::TnaReady {
                    targeted: false,
                    report_id: None,
                    scope: None,
                    result,
                });
            });
            return;
        }
        if let Err(err) = tna::rebuild_after_delete(&self.store, report_id) {
            self.log_event("task", &format!("tna rebuild failed: {err}"));
            return;
        }
        self.tna_report = None;
        if let Ok(Some(snap)) = self.store.get_tna_graph(desk_key()) {
            self.tna_desk = Some(snap);
        } else {
            self.tna_desk = None;
        }
    }

    fn dismiss_tna_answer(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        self.turn_generation = self.turn_generation.wrapping_add(1);
        self.running = false;
        self.tna_answer = None;
        self.tna_answer_scroll = 0;
        self.status = "ready".into();
    }

    fn close_report_workspace(&mut self) {
        self.dismiss_tna_answer();
        self.chat_report = None;
        self.chat_case = None;
        self.tna_report = None;
        self.tna_pending_report = None;
        self.tna_rebuilding = false;
        self.tna_source.clear();
        self.tna_source_error = None;
        self.tna_find = None;
        self.tna_find_editing = false;
        self.tna_from = None;
        self.tna_to = None;
        self.tna_tab_hits.clear();
        *self.tna_path_cache.borrow_mut() = None;
        self.tna_table_list_area = Rect::default();
        self.tna_table_detail_area = Rect::default();
        self.module = Some(ModuleId::Cases);
        self.case_page = CasePage::Closed;
        self.focus = Focus::Prompt;
        self.prompt.clear();
        self.cursor = 0;
        self.load_transcript("desk");
        self.scroll_back = self.desk_return_scroll;
        self.prompt = std::mem::take(&mut self.desk_return_prompt);
        self.cursor = self.prompt.chars().count();
        self.evidence_scope = self.desk_return_scope.clone();
        self.workspace_reading = false;
    }

    fn on_tna_turn(&mut self, ev: TurnEvent) {
        if self.tna_answer.is_none() {
            return;
        }
        match ev {
            TurnEvent::Status(text) => self.status = text,
            TurnEvent::Delta(text) => {
                if let Some(answer) = self.tna_answer.as_mut().filter(|a| a.pending) {
                    answer.answer.push_str(&text);
                }
            }
            TurnEvent::Note(text) => self.log_event(note_kind(&text), &text),
            TurnEvent::Done(text) => {
                if self.cancel.load(Ordering::Relaxed)
                    || !self.tna_answer.as_ref().is_some_and(|a| a.pending)
                {
                    return;
                }
                if text.trim().is_empty() {
                    self.on_tna_turn(TurnEvent::Failed(
                        "Provider returned an empty answer.".into(),
                    ));
                    return;
                }
                self.running = false;
                self.status = "ready".into();
                if let Some(answer) = self.tna_answer.as_mut() {
                    answer.pending = false;
                    answer.answer = text.clone();
                }
                // Discussion remains ephemeral. Saving knowledge is an explicit analyst action.
            }
            TurnEvent::Failed(err) => {
                self.running = false;
                self.status = "answer error".into();
                self.log_event("api", &err);
                if let Some(answer) = self.tna_answer.as_mut() {
                    answer.pending = false;
                    answer.error = Some(err);
                }
            }
            // Evidence-only turns cannot write raw memories or file reports.
            TurnEvent::Memory(_) | TurnEvent::Report(_) => {}
        }
    }

    pub fn tna_focus_node(&self) -> Option<&TnaNode> {
        let snap = self.tna_snapshot()?;
        self.tna_focus_id
            .as_ref()
            .and_then(|id| snap.nodes.iter().find(|n| &n.id == id))
            .or_else(|| {
                self.tna_visible_nodes()
                    .get(self.tna_sel)
                    .and_then(|i| snap.nodes.get(*i))
            })
    }

    pub fn set_tna_layout(&mut self, layout: TnaLayout) {
        self.tna_layout = layout;
        self.focus = Focus::Graph;
        self.tna_path_sel = 0;
        self.tna_hop_sel = 0;
    }

    pub fn tna_focus_entity(&mut self, id: &str) {
        if let Some(index) = self
            .tna_visible_nodes()
            .iter()
            .position(|i| self.tna_snapshot().is_some_and(|s| s.nodes[*i].id == id))
        {
            self.tna_sel = index;
        }
        self.tna_focus_id = Some(id.into());
        self.tna_detail_scroll = 0;
        self.tna_ledger_sel = 0;
        self.sync_tna_table_ui();
    }

    fn focus_selected_passage(&mut self) {
        let Some((report_id, start, end, _)) = &self.selected_passage else {
            return;
        };
        let Some(snapshot) = self.tna_report.as_ref() else {
            return;
        };
        let entity = snapshot
            .decisions
            .iter()
            .find(|d| {
                d.report_id == *report_id
                    && d.start >= *start
                    && d.start < *end
                    && self.tna_source.get(d.start..d.end) == Some(d.original.as_str())
                    && self
                        .report_read_source
                        .as_deref()
                        .unwrap_or(&self.tna_source)
                        .get(d.start..d.end)
                        == Some(d.original.as_str())
            })
            .and_then(|d| d.canonical_id.clone());
        if let Some(id) = entity {
            self.tna_focus_entity(&id);
        }
    }

    fn workspace_key(&mut self, key: KeyEvent) -> bool {
        let code = key.code;
        let layout = match code {
            KeyCode::Char('g') => Some(TnaLayout::Cockpit),
            KeyCode::Char('q') => Some(TnaLayout::Clusters),
            KeyCode::Char('p') => Some(TnaLayout::Path),
            KeyCode::Char('m') => Some(TnaLayout::Matrix),
            KeyCode::Char('r') => Some(TnaLayout::Ribbon),
            KeyCode::Left => Some(TnaLayout::ALL[(self.tna_layout.index() + 4) % 5]),
            KeyCode::Right => Some(TnaLayout::ALL[(self.tna_layout.index() + 1) % 5]),
            _ => None,
        };
        if let Some(layout) = layout {
            self.set_tna_layout(layout);
            return true;
        }
        match code {
            KeyCode::Char('?') => {
                self.help = true;
                return true;
            }
            KeyCode::PageUp => {
                self.tna_answer_scroll = self.tna_answer_scroll.saturating_sub(6);
                return true;
            }
            KeyCode::PageDown => {
                self.tna_answer_scroll = self.tna_answer_scroll.saturating_add(6);
                return true;
            }
            KeyCode::Enter if self.tna_answer.is_some() => {
                self.focus = Focus::Prompt;
                return true;
            }
            KeyCode::Enter if self.tna_layout == TnaLayout::Clusters => {
                self.set_tna_layout(TnaLayout::Cockpit);
                return true;
            }
            KeyCode::Char('[') | KeyCode::Char(']') if self.tna_layout == TnaLayout::Cockpit => {
                let neighbors = self.tna_links();
                if !neighbors.is_empty() {
                    self.tna_ledger_sel = if code == KeyCode::Char(']') {
                        (self.tna_ledger_sel + 1) % neighbors.len()
                    } else {
                        (self.tna_ledger_sel + neighbors.len() - 1) % neighbors.len()
                    };
                    self.tna_detail_scroll = 0;
                }
                return true;
            }
            _ => {}
        }
        match self.tna_layout {
            TnaLayout::Clusters if self.focus == Focus::TableDetail => {
                if matches!(code, KeyCode::Up | KeyCode::Down | KeyCode::Char('j' | 'k')) {
                    let ids: Vec<_> = self
                        .tna_snapshot()
                        .into_iter()
                        .flat_map(|s| &s.anchors)
                        .map(|a| a.node_id.clone())
                        .collect();
                    if !ids.is_empty() {
                        let current = ids
                            .iter()
                            .position(|id| Some(id) == self.tna_focus_id.as_ref())
                            .unwrap_or(0);
                        let index = if matches!(code, KeyCode::Up | KeyCode::Char('k')) {
                            (current + ids.len() - 1) % ids.len()
                        } else {
                            (current + 1) % ids.len()
                        };
                        self.tna_focus_entity(&ids[index]);
                    }
                    return true;
                }
            }
            TnaLayout::Path => match code {
                KeyCode::Char('f') | KeyCode::Char('t') => {
                    let id = self.tna_focus_node().map(|n| n.id.clone());
                    if code == KeyCode::Char('f') {
                        self.tna_from = id;
                    } else {
                        self.tna_to = id;
                    }
                    self.tna_path_sel = 0;
                    self.tna_hop_sel = 0;
                    return true;
                }
                KeyCode::Up | KeyCode::Char('k') if self.focus == Focus::TableDetail => {
                    self.tna_path_sel = self.tna_path_sel.saturating_sub(1);
                    return true;
                }
                KeyCode::Down | KeyCode::Char('j') if self.focus == Focus::TableDetail => {
                    self.tna_path_sel =
                        (self.tna_path_sel + 1).min(self.tna_paths().len().saturating_sub(1));
                    return true;
                }
                _ => {}
            },
            TnaLayout::Matrix => {
                let len = self.tna_matrix_nodes().len();
                if len > 0 {
                    self.tna_matrix_row = self.tna_matrix_row.min(len - 1);
                    self.tna_matrix_col = self.tna_matrix_col.min(len - 1);
                    match code {
                        KeyCode::Char('h') => {
                            self.tna_matrix_col = self.tna_matrix_col.saturating_sub(1)
                        }
                        KeyCode::Char('l') => {
                            self.tna_matrix_col = (self.tna_matrix_col + 1).min(len - 1)
                        }
                        KeyCode::Char('k') | KeyCode::Up => {
                            self.tna_matrix_row = self.tna_matrix_row.saturating_sub(1)
                        }
                        KeyCode::Char('j') | KeyCode::Down => {
                            self.tna_matrix_row = (self.tna_matrix_row + 1).min(len - 1)
                        }
                        KeyCode::Enter => {
                            let index = self.tna_matrix_nodes()[self.tna_matrix_row];
                            if let Some(node) = self.tna_snapshot().and_then(|s| s.nodes.get(index))
                            {
                                let id = node.id.clone();
                                let other = self
                                    .tna_matrix_nodes()
                                    .get(self.tna_matrix_col)
                                    .and_then(|i| self.tna_snapshot().and_then(|s| s.nodes.get(*i)))
                                    .map(|n| n.id.clone());
                                self.tna_focus_entity(&id);
                                self.set_tna_layout(TnaLayout::Cockpit);
                                if let Some(other) = other {
                                    self.tna_ledger_sel = self
                                        .tna_links()
                                        .iter()
                                        .position(|e| e.from == other || e.to == other)
                                        .unwrap_or(0);
                                }
                            }
                        }
                        _ => return false,
                    }
                }
                return matches!(
                    code,
                    KeyCode::Char('h' | 'j' | 'k' | 'l')
                        | KeyCode::Up
                        | KeyCode::Down
                        | KeyCode::Enter
                );
            }
            TnaLayout::Ribbon => match code {
                KeyCode::Char('d') => {
                    self.tna_show_rejected = !self.tna_show_rejected;
                    return true;
                }
                KeyCode::Char('h' | 'k') | KeyCode::Up => {
                    self.tna_ribbon_pos = self.tna_ribbon_pos.saturating_sub(1);
                    return true;
                }
                KeyCode::Char('l' | 'j') | KeyCode::Down => {
                    self.tna_ribbon_pos = (self.tna_ribbon_pos + 1)
                        .min(self.tna_ribbon_positions().len().saturating_sub(1));
                    return true;
                }
                _ => {}
            },
            _ => {}
        }
        false
    }

    pub fn tna_links(&self) -> Vec<&argos_osint_core::tna::TnaEdge> {
        let Some(snap) = self.tna_snapshot() else {
            return Vec::new();
        };
        let Some(node) = self.tna_focus_node() else {
            return Vec::new();
        };
        let mut edges: Vec<_> = snap
            .edges
            .iter()
            .filter(|e| e.from == node.id || e.to == node.id)
            .collect();
        edges.sort_by(|a, b| {
            b.weight
                .cmp(&a.weight)
                .then(a.from.cmp(&b.from))
                .then(a.to.cmp(&b.to))
        });
        edges
    }

    pub fn tna_label(&self, id: &str) -> String {
        self.tna_snapshot()
            .and_then(|s| s.nodes.iter().find(|n| n.id == id))
            .map(|n| n.label.clone())
            .unwrap_or_else(|| id.into())
    }

    pub fn tna_edge_weight(&self, a: &str, b: &str) -> u32 {
        self.tna_snapshot()
            .and_then(|s| {
                s.edges
                    .iter()
                    .find(|e| (e.from == a && e.to == b) || (e.to == a && e.from == b))
            })
            .map(|e| e.weight)
            .unwrap_or(0)
    }

    pub fn tna_edge_evidence(&self, a: &str, b: &str) -> String {
        if self.tna_snapshot().is_none() {
            return "Evidence unavailable.".into();
        }
        let accepted = self.tna_accepted_mentions();
        // Match the existing three-mention window, including the two-mention case.
        for (i, d) in accepted.iter().enumerate() {
            if d.canonical_id.as_deref() != Some(a) {
                continue;
            }
            for (_, other) in accepted
                .iter()
                .enumerate()
                .filter(|(j, _)| i.abs_diff(*j) <= 2)
            {
                if other.canonical_id.as_deref() != Some(b) {
                    continue;
                }
                if d.report_id != other.report_id
                    || self
                        .tna_source
                        .get(d.start.min(other.start)..d.start.max(other.start))
                        .is_none_or(|s| s.contains('\n'))
                {
                    continue;
                }
                let start = d.start.min(other.start);
                let end = d.end.max(other.end);
                if let Some(text) = self.tna_source.get(start..end).filter(|_| {
                    self.tna_source.get(d.start..d.end) == Some(d.original.as_str())
                        && self.tna_source.get(other.start..other.end)
                            == Some(other.original.as_str())
                }) {
                    let excerpt: String = text
                        .split_whitespace()
                        .collect::<Vec<_>>()
                        .join(" ")
                        .chars()
                        .take(360)
                        .collect();
                    return format!(
                        "Text co-occurrence · confidence unassessed · not ownership/identity\n{} · {} · bytes {}–{}\n{}",
                        d.report_id, d.section, start, end, excerpt
                    );
                }
                return format!(
                    "{} · {} · bytes {}–{}\n{}: {} / {}: {}",
                    d.report_id,
                    d.section,
                    start,
                    end,
                    d.original,
                    d.reason,
                    other.original,
                    other.reason
                );
            }
        }
        let reasons: Vec<_> = accepted
            .iter()
            .filter(|d| matches!(d.canonical_id.as_deref(), Some(id) if id == a || id == b))
            .take(2)
            .map(|d| {
                format!(
                    "{} · {} · bytes {}–{}: {} — {}",
                    d.report_id, d.section, d.start, d.end, d.original, d.reason
                )
            })
            .collect();
        if reasons.is_empty() {
            "Evidence unavailable in this snapshot; edge weight is existing co-occurrence, not a verified relationship.".into()
        } else {
            format!("No joint excerpt recovered.\n{}", reasons.join("\n"))
        }
    }

    pub fn tna_matrix_nodes(&self) -> Vec<usize> {
        let Some(snap) = self.tna_snapshot() else {
            return Vec::new();
        };
        let mut nodes = self.tna_visible_nodes();
        nodes.sort_by(|a, b| {
            snap.nodes[*b]
                .degree
                .cmp(&snap.nodes[*a].degree)
                .then(snap.nodes[*a].id.cmp(&snap.nodes[*b].id))
        });
        nodes.truncate(24);
        nodes
    }

    pub fn tna_cluster_counts(&self) -> [[u32; 4]; 4] {
        let mut counts = [[0; 4]; 4];
        if let Some(snap) = self.tna_snapshot() {
            for edge in &snap.edges {
                let a = snap
                    .nodes
                    .iter()
                    .find(|n| n.id == edge.from)
                    .map(|n| n.cluster);
                let b = snap
                    .nodes
                    .iter()
                    .find(|n| n.id == edge.to)
                    .map(|n| n.cluster);
                if let (Some(a), Some(b)) = (a, b) {
                    let i = TnaCluster::all().iter().position(|c| *c == a).unwrap_or(0);
                    let j = TnaCluster::all().iter().position(|c| *c == b).unwrap_or(0);
                    counts[i][j] += 1;
                    if i != j {
                        counts[j][i] += 1;
                    }
                }
            }
        }
        counts
    }

    pub fn tna_paths(&self) -> Vec<TnaPath> {
        let (Some(from), Some(to), Some(snap)) = (
            self.investigation
                .as_ref()
                .map(|w| &w.path_from)
                .unwrap_or(&self.tna_from),
            self.investigation
                .as_ref()
                .map(|w| &w.path_to)
                .unwrap_or(&self.tna_to),
            self.tna_snapshot(),
        ) else {
            return Vec::new();
        };
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        for edge in &snap.edges {
            (&edge.from, &edge.to, edge.weight).hash(&mut hash);
        }
        let fingerprint = hash.finish();
        if let Some((a, b, version, paths)) = &*self.tna_path_cache.borrow() {
            if a == from && b == to && *version == fingerprint {
                return paths.clone();
            }
        }
        let paths = self.find_tna_paths();
        *self.tna_path_cache.borrow_mut() =
            Some((from.clone(), to.clone(), fingerprint, paths.clone()));
        paths
    }

    fn find_tna_paths(&self) -> Vec<TnaPath> {
        let (Some(from), Some(to), Some(snap)) = (
            self.investigation
                .as_ref()
                .map(|w| &w.path_from)
                .unwrap_or(&self.tna_from),
            self.investigation
                .as_ref()
                .map(|w| &w.path_to)
                .unwrap_or(&self.tna_to),
            self.tna_snapshot(),
        ) else {
            return Vec::new();
        };
        self.tna_path_search_limited.set(false);
        if from == to {
            return vec![TnaPath {
                nodes: vec![from.clone()],
                strength: 0,
            }];
        }
        let mut adj: HashMap<&str, Vec<(&str, u32)>> = HashMap::new();
        let mut max_weight = 0;
        for edge in &snap.edges {
            adj.entry(&edge.from)
                .or_default()
                .push((&edge.to, edge.weight));
            adj.entry(&edge.to)
                .or_default()
                .push((&edge.from, edge.weight));
            max_weight = max_weight.max(edge.weight);
        }
        let mut distances = HashMap::from([(to.as_str(), 0usize)]);
        let mut frontier = VecDeque::from([to.as_str()]);
        while let Some(id) = frontier.pop_front() {
            let distance = distances[id];
            if distance == 4 {
                continue;
            }
            for (next, _) in adj.get(id).into_iter().flatten() {
                if !distances.contains_key(next) {
                    distances.insert(*next, distance + 1);
                    frontier.push_back(*next);
                }
            }
        }
        if !distances.contains_key(from.as_str()) {
            return Vec::new();
        }
        let mut results = Vec::new();
        let mut expanded = 0;
        // Upper-bound priority finds stronger equal-length paths first. Bound work
        // for dense graphs; no new graph metrics or persisted data are introduced.
        for hops in 1..=4usize {
            let mut queue = std::collections::BinaryHeap::new();
            queue.push((max_weight as u64 * hops as u64, 0u32, vec![from.clone()]));
            while let Some((_, strength, nodes)) = queue.pop() {
                expanded += 1;
                if expanded > 20_000 {
                    self.tna_path_search_limited.set(true);
                    return results;
                }
                let last = nodes.last().unwrap();
                if last == to {
                    if nodes.len() == hops + 1 {
                        results.push(TnaPath { nodes, strength });
                    }
                    if results.len() == 5 {
                        return results;
                    }
                    continue;
                }
                if nodes.len() > hops {
                    continue;
                }
                for (next, weight) in adj.get(last.as_str()).into_iter().flatten() {
                    let remaining = hops + 1 - (nodes.len() + 1);
                    if nodes.iter().any(|n| n == next)
                        || distances
                            .get(next)
                            .is_none_or(|distance| *distance > remaining)
                    {
                        continue;
                    }
                    if queue.len() >= 20_000 {
                        self.tna_path_search_limited.set(true);
                        return results;
                    }
                    let mut next_nodes = nodes.clone();
                    next_nodes.push((*next).into());
                    let next_strength = strength.saturating_add(*weight);
                    let remaining = hops + 1 - next_nodes.len();
                    queue.push((
                        next_strength as u64 + max_weight as u64 * remaining as u64,
                        next_strength,
                        next_nodes,
                    ));
                }
            }
        }
        results
    }

    /// Scrub source lines, including sections with no extracted candidates.
    pub fn tna_ribbon_positions(&self) -> Vec<usize> {
        let mut positions = Vec::new();
        let mut offset = 0;
        for line in self.tna_source.split_inclusive('\n') {
            if !line.trim().is_empty() {
                positions.push(offset);
            }
            offset += line.len();
        }
        positions
    }

    fn tna_report_material(&self, report: &ReportMeta) -> String {
        let prefix: String = self.tna_source.chars().take(3500).collect();
        let mut out = format!(
            "OPEN REPORT: {} ({})\n{}\n",
            report.title, report.id, prefix
        );
        if prefix.len() < self.tna_source.len() {
            out.push_str("[Report prefix clipped; additional focused evidence follows. Absence from these excerpts does not establish absence from the full report.]\n");
        }
        let mut ids: Vec<&str> = self
            .tna_focus_node()
            .map(|n| n.id.as_str())
            .into_iter()
            .collect();
        if self.tna_layout == TnaLayout::Path {
            ids.extend(self.tna_from.as_deref());
            ids.extend(self.tna_to.as_deref());
        }
        let mut seen = std::collections::HashSet::new();
        for d in self
            .tna_accepted_mentions()
            .into_iter()
            .filter(|d| {
                d.canonical_id
                    .as_deref()
                    .is_some_and(|id| ids.contains(&id))
            })
            .take(4)
        {
            if !seen.insert(d.start) {
                continue;
            }
            out.push_str(&format!(
                "\nFocused source: {} · bytes {}–{}\n{}\n",
                d.section,
                d.start,
                d.end,
                self.tna_source_around(d.start, 500)
            ));
        }
        if self.tna_layout == TnaLayout::Ribbon {
            if let Some(position) = self.tna_ribbon_positions().get(self.tna_ribbon_pos) {
                out.push_str(&format!(
                    "\nRibbon source at byte {position}:\n{}",
                    self.tna_source_around(*position, 700)
                ));
            }
        }
        out
    }

    fn tna_source_around(&self, position: usize, radius: usize) -> &str {
        let mut start = position.saturating_sub(radius).min(self.tna_source.len());
        let mut end = position.saturating_add(radius).min(self.tna_source.len());
        while !self.tna_source.is_char_boundary(start) {
            start = start.saturating_sub(1);
        }
        while !self.tna_source.is_char_boundary(end) {
            end = end.saturating_sub(1);
        }
        self.tna_source.get(start..end).unwrap_or("")
    }

    pub fn tna_ribbon_decisions(&self) -> Vec<&argos_osint_core::tna::TnaDecision> {
        let mut decisions: Vec<_> = self
            .tna_snapshot()
            .into_iter()
            .flat_map(|s| &s.decisions)
            .filter(|d| {
                Some(d.report_id.as_str()) == self.chat_report.as_deref()
                    && (self.tna_show_rejected || d.canonical_id.is_some())
            })
            .collect();
        decisions.sort_by_key(|d| (d.start, d.end));
        decisions
    }

    fn tna_accepted_mentions(&self) -> Vec<&argos_osint_core::tna::TnaDecision> {
        let mut decisions: Vec<_> = self
            .tna_snapshot()
            .into_iter()
            .flat_map(|s| &s.decisions)
            .filter(|d| {
                d.canonical_id.is_some()
                    && Some(d.report_id.as_str()) == self.chat_report.as_deref()
            })
            .collect();
        decisions.sort_by_key(|d| d.start);
        let mut seen = std::collections::HashSet::new();
        decisions.retain(|d| seen.insert((d.start, d.canonical_id.as_deref())));
        decisions
    }

    pub fn tna_ledger_neighbor(&self) -> Option<&str> {
        let focus = self.tna_focus_node()?;
        let edges = self.tna_links();
        let edge = edges.get(self.tna_ledger_sel.min(edges.len().saturating_sub(1)))?;
        Some(if edge.from == focus.id {
            &edge.to
        } else {
            &edge.from
        })
    }

    pub fn tna_ribbon_window(&self, position: usize) -> Vec<&argos_osint_core::tna::TnaDecision> {
        let accepted = self.tna_accepted_mentions();
        let nearest = accepted
            .iter()
            .enumerate()
            .min_by_key(|(_, d)| d.start.abs_diff(position))
            .map(|(i, _)| i)
            .unwrap_or(0);
        let start = nearest
            .saturating_sub(1)
            .min(accepted.len().saturating_sub(3));
        accepted.into_iter().skip(start).take(3).collect()
    }

    fn tna_digest(&self) -> String {
        let mut lines = vec![format!("Layout: {}", self.tna_layout.title())];
        if let Some(node) = self.tna_focus_node() {
            lines.push(format!(
                "Selected: {} ({}, degree {})",
                node.label,
                node.kind.as_str(),
                node.degree
            ));
            let neighbors: Vec<_> = self
                .tna_links()
                .iter()
                .take(8)
                .map(|e| {
                    format!(
                        "{} [w{}]",
                        self.tna_label(if e.from == node.id { &e.to } else { &e.from }),
                        e.weight
                    )
                })
                .collect();
            lines.push(format!("Neighbors: {}", neighbors.join(", ")));
        }
        if self.tna_layout == TnaLayout::Path {
            lines.push(format!(
                "Path FROM: {}; TO: {}",
                self.tna_from
                    .as_ref()
                    .map(|id| self.tna_label(id))
                    .unwrap_or_else(|| "unset".into()),
                self.tna_to
                    .as_ref()
                    .map(|id| self.tna_label(id))
                    .unwrap_or_else(|| "unset".into())
            ));
        }
        if let Some(snap) = self.tna_snapshot() {
            for gap in snap.gaps.iter().take(3) {
                lines.push(format!(
                    "No co-occurrence: {} / {}",
                    gap.cluster_a.label(),
                    gap.cluster_b.label()
                ));
            }
        }
        lines.join("\n").chars().take(1600).collect()
    }

    fn on_tna_key(&mut self, key: KeyEvent) -> bool {
        if self.tna_layout == TnaLayout::Matrix && self.focus != Focus::Prompt {
            if key.code == KeyCode::Char('c') {
                self.tna_matrix_coverage = !self.tna_matrix_coverage;
                self.tna_matrix_row = 0;
                self.tna_matrix_col = 0;
                if self.tna_matrix_coverage {
                    self.load_coverage();
                }
                return false;
            }
            if self.tna_matrix_coverage {
                let rows = self.coverage_rows.len();
                let columns = self
                    .coverage_rows
                    .first()
                    .map(|r| r.cells.len())
                    .unwrap_or(0);
                match key.code {
                    KeyCode::Char('j') | KeyCode::Down => {
                        self.tna_matrix_row = (self.tna_matrix_row + 1).min(rows.saturating_sub(1))
                    }
                    KeyCode::Char('k') | KeyCode::Up => {
                        self.tna_matrix_row = self.tna_matrix_row.saturating_sub(1)
                    }
                    KeyCode::Char('l') => {
                        self.tna_matrix_col =
                            (self.tna_matrix_col + 1).min(columns.saturating_sub(1))
                    }
                    KeyCode::Char('h') => {
                        self.tna_matrix_col = self.tna_matrix_col.saturating_sub(1)
                    }
                    KeyCode::Enter => self.open_coverage_passage(),
                    _ => {}
                }
                if matches!(
                    key.code,
                    KeyCode::Char('h' | 'j' | 'k' | 'l')
                        | KeyCode::Up
                        | KeyCode::Down
                        | KeyCode::Enter
                ) {
                    return false;
                }
            }
        }
        if self.tna_layout == TnaLayout::Path
            && self.focus != Focus::Prompt
            && matches!(key.code, KeyCode::Char('[' | ']'))
        {
            let hops = self
                .tna_paths()
                .get(self.tna_path_sel)
                .map(|p| p.nodes.len().saturating_sub(1))
                .unwrap_or(0);
            if hops > 0 {
                self.tna_hop_sel = if key.code == KeyCode::Char('[') {
                    (self.tna_hop_sel + hops - 1) % hops
                } else {
                    (self.tna_hop_sel + 1) % hops
                };
            }
            return false;
        }
        if key.code == KeyCode::Char('i') && self.focus != Focus::Prompt {
            self.show_evidence_surface("inspect", "");
            return false;
        }
        if key.code == KeyCode::Char('R') && self.focus != Focus::Prompt {
            self.workspace_reading = true;
            return false;
        }
        if self.workspace_reading && self.focus != Focus::Prompt {
            match key.code {
                KeyCode::Up | KeyCode::Char('k') => {
                    self.report_read_line = self.report_read_line.saturating_sub(1);
                    return false;
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    self.report_read_line = (self.report_read_line + 1).min(
                        self.report_read_source
                            .as_deref()
                            .unwrap_or(&self.tna_source)
                            .lines()
                            .count()
                            .saturating_sub(1),
                    );
                    return false;
                }
                KeyCode::Char('g') => {
                    self.workspace_reading = false;
                    self.set_tna_layout(TnaLayout::Cockpit);
                    return false;
                }
                _ => {}
            }
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            return false;
        }
        if self.tna_find_editing {
            match key.code {
                KeyCode::Esc => {
                    self.tna_find = None;
                    self.tna_find_editing = false;
                }
                KeyCode::Backspace => {
                    if let Some(q) = self.tna_find.as_mut() {
                        q.pop();
                    }
                }
                KeyCode::Enter => {
                    self.tna_find_editing = false;
                }
                KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    if let Some(q) = self.tna_find.as_mut() {
                        q.push(c);
                    }
                }
                _ => {}
            }
            self.clamp_tna_sel();
            self.sync_tna_table_ui();
            return false;
        }
        if self.workspace_key(key) {
            return false;
        }
        match key.code {
            KeyCode::Char('/') => {
                self.tna_find = Some(String::new());
                self.tna_find_editing = true;
                self.focus = Focus::Graph;
            }
            KeyCode::Enter => {
                self.pin_tna_selection(self.tna_selected_item(), true);
            }
            KeyCode::Char('k') | KeyCode::Up => {
                if self.focus == Focus::TableDetail {
                    self.tna_detail_scroll = self.tna_detail_scroll.saturating_sub(1);
                    self.sync_tna_detail_scroll_state();
                } else {
                    self.tna_step_sel(-1);
                }
            }
            KeyCode::Char('j') | KeyCode::Down => {
                if self.focus == Focus::TableDetail {
                    self.tna_detail_scroll = self.tna_detail_scroll.saturating_add(1);
                    self.sync_tna_detail_scroll_state();
                } else {
                    self.tna_step_sel(1);
                }
            }
            KeyCode::Tab => {
                self.focus = self.next_focus();
            }
            _ => {}
        }
        false
    }

    fn tna_step_sel(&mut self, delta: i32) {
        let n = self.tna_selectable_count();
        if n == 0 {
            self.tna_sel = 0;
            self.sync_tna_table_ui();
            return;
        }
        let cur = self.tna_sel as i32;
        let next = (cur + delta).rem_euclid(n as i32) as usize;
        self.tna_sel = next;
        self.tna_detail_scroll = 0;
        // Outline/Table: keep focus id in sync for when user switches back to Graph.
        if let Some(TnaDisplayItem::Real { idx }) = self.tna_selected_item() {
            if let Some(snap) = self.tna_snapshot() {
                if let Some(n) = snap.nodes.get(idx) {
                    self.tna_focus_id = Some(n.id.clone());
                }
            }
        }
        self.sync_tna_table_ui();
    }

    fn on_esc(&mut self) {
        if self.investigation.is_some() {
            self.close_investigation();
            return;
        }
        if self.modal {
            self.modal = false;
            return;
        }
        if self.editing {
            self.editing = false;
            return;
        }
        if self.chat_report.is_some() {
            if self.tna_find.is_some() {
                self.tna_find = None;
                self.tna_find_editing = false;
                self.clamp_tna_sel();
                return;
            }
            if self.tna_answer.is_some() {
                self.dismiss_tna_answer();
                return;
            }
            self.close_report_workspace();
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
            self.sync_brain_list_ui();
            return;
        }
        self.brain_sel = (self.brain_sel as isize + delta).rem_euclid(n as isize) as usize;
        self.sync_brain_list_ui();
    }

    const BRAIN_LIST_ITEM_HEIGHT: usize = 1;

    /// Keep `ListState` / memory-list scrollbar aligned with `brain_sel`.
    pub fn sync_brain_list_ui(&mut self) {
        let n = self.shown_memories().len();
        if n == 0 {
            self.brain_sel = 0;
            self.brain_list_state.select(None);
            self.brain_list_scroll = ScrollbarState::new(0).position(0);
            return;
        }
        if self.brain_sel >= n {
            self.brain_sel = n - 1;
        }
        let i = self.brain_sel;
        self.brain_list_state.select(Some(i));
        let content = n.saturating_sub(1) * Self::BRAIN_LIST_ITEM_HEIGHT;
        self.brain_list_scroll =
            ScrollbarState::new(content).position(i * Self::BRAIN_LIST_ITEM_HEIGHT);
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
            existing_case: None,
            include_evidence: false,
            allow_sensitive: false,
            allow_active: false,
            query,
            echo_on_desk,
            restore_prompt,
            facts: false,
            web: false,
            news: false,
            domain: false,
            social: false,
            identity: false,
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
            KeyCode::Char('c') => {
                if let Some(scope) = self.scope.as_mut() {
                    scope.existing_case = match scope
                        .existing_case
                        .as_ref()
                        .and_then(|id| self.cases.iter().position(|c| &c.id == id))
                    {
                        Some(index) => self.cases.get(index + 1).map(|c| c.id.clone()),
                        None => self.cases.first().map(|c| c.id.clone()),
                    };
                }
            }
            KeyCode::Char('e') => {
                if let Some(scope) = self.scope.as_mut() {
                    scope.include_evidence = !scope.include_evidence;
                }
            }
            KeyCode::Char('s') => {
                if let Some(scope) = self.scope.as_mut() {
                    scope.allow_sensitive = !scope.allow_sensitive;
                }
            }
            KeyCode::Char('a') => {
                if let Some(scope) = self.scope.as_mut() {
                    scope.allow_active = !scope.allow_active;
                }
            }
            KeyCode::Enter => {
                if let Some(scope) = self.scope.take() {
                    let plan = self.plan_for_scope(&scope);
                    if scope.restore_prompt {
                        self.prompt = scope.query.clone();
                        self.cursor = self.prompt.chars().count();
                    }
                    self.start_case_workspace(
                        scope.query,
                        scope.echo_on_desk,
                        plan,
                        scope.existing_case,
                        scope.include_evidence,
                        scope.allow_sensitive,
                        scope.allow_active,
                    );
                }
            }
            KeyCode::Esc => {
                if let Some(scope) = self.scope.take() {
                    if scope.restore_prompt {
                        self.prompt = scope.query;
                        self.cursor = self.prompt.chars().count();
                        self.focus = Focus::Prompt;
                    }
                    self.status = "Investigation scope cancelled".into();
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

    #[cfg(test)]
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
                let _ = self.store.add_report_metadata(&report);
                let _ = self.reload_lists();
                self.after_report_filed(&report.id);
            }
            Ok((summary, None)) => self.mark_task_failed(&case_id, &summary),
            Err(err) => self.mark_task_failed(&case_id, &err),
        }
    }

    #[cfg(test)]
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
        self.prepare_case_data(&case_id, true);
    }

    fn save_osint_toggles(&mut self) {
        self.settings.facts = self.field_value("facts").eq_ignore_ascii_case("yes");
        self.settings.web = self.field_value("web").eq_ignore_ascii_case("yes");
        self.settings.news = self.field_value("news").eq_ignore_ascii_case("yes");
        self.settings.domain = self.field_value("domain").eq_ignore_ascii_case("yes");
        self.settings.social = self.field_value("social").eq_ignore_ascii_case("yes");
        self.settings.identity = self.field_value("identity").eq_ignore_ascii_case("yes");
        self.settings.searx_url = self.field_value("searx_url");
        for (field, name) in [
            ("brave_key", "BRAVE_API_KEY"),
            ("tavily_key", "TAVILY_API_KEY"),
            ("youtube_key", "YOUTUBE_API_KEY"),
            ("github_token", "GITHUB_TOKEN"),
        ] {
            let value = self.field_value(field);
            if value.is_empty() {
                self.auth.research.remove(name);
            } else {
                self.auth.research.insert(name.into(), value);
            }
        }
        if let Err(err) = self.save_auth_config(&self.auth) {
            self.status = format!("Secret save failed: {err}");
            return;
        }
        self.settings.brave_key.clear();
        self.settings.tavily_key.clear();
        self.settings.youtube_key.clear();
        self.settings.github_token.clear();
        match self.save_settings_config(&self.settings) {
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
        if let Err(err) = self.save_settings_config(&self.settings) {
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
        let _ = self.save_settings_config(&self.settings);
    }

    fn toggle_osint_source(&mut self) {
        if let Some(source) = self.settings.sources.get_mut(self.source_sel) {
            source.enabled = !source.enabled;
        }
        let _ = self.save_settings_config(&self.settings);
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
        if self.chat_report.is_none() {
            self.history.push(line.clone());
        }
        self.hist_pos = None;
        self.prompt.clear();
        self.cursor = 0;
        if line.starts_with('/') {
            self.run_slash(&line);
        } else if self.routes_desk_message() {
            self.route_desk_message(line);
        } else if self.chat_report.is_some() || self.chat_case.is_some() {
            self.retrieve_question(line);
        } else {
            self.spawn_turn(line);
        }
    }

    fn routes_desk_message(&self) -> bool {
        self.on_case_desk() && self.chat_case.is_none() && self.chat_report.is_none()
    }

    fn open_report_chat(&mut self, id: &str) {
        if self.investigation.is_some() {
            self.close_investigation();
        }
        let Some(report) = self.reports.iter().find(|report| report.id == id).cloned() else {
            self.status = "that report is no longer on file".into();
            return;
        };
        if self.chat_report.is_none() {
            self.originating_question = self
                .transcripts
                .get(&self.session_id())
                .and_then(|lines| {
                    lines
                        .iter()
                        .rev()
                        .find(|line| line.role == "user" && !line.body.starts_with('/'))
                })
                .map(|line| line.body.clone())
                .unwrap_or_default();
        }
        self.persist_visible_chat();
        self.cancel.store(true, Ordering::Relaxed);
        self.turn_generation = self.turn_generation.wrapping_add(1);
        self.report_source_generation = self.report_source_generation.wrapping_add(1);
        self.running = false;
        if self.chat_report.as_deref() != Some(id) {
            self.tna_report = None;
        }
        if self.chat_report.is_none() {
            self.desk_return_scope = self.evidence_scope.clone();
            self.desk_return_scroll = self.scroll_back;
            self.desk_return_prompt = self.prompt.clone();
        }
        self.chat_case = None;
        self.chat_report = Some(report.id.clone());
        self.evidence_scope = argos_osint_core::evidence::EvidenceScope::Report(report.id.clone());
        self.workspace_reading = false;
        self.report_read_line = 0;
        self.selected_passage = None;
        self.report_source_version = None;
        self.report_read_source = None;
        self.case_page = CasePage::Network;
        self.module = Some(ModuleId::Cases);
        self.focus = Focus::Graph;
        self.brain_card = false;
        self.editing = false;
        self.tna_layout = TnaLayout::Cockpit;
        self.tna_answer = None;
        self.tna_from = None;
        self.tna_to = None;
        self.tna_path_sel = 0;
        self.tna_hop_sel = 0;
        self.tna_matrix_row = 0;
        self.tna_matrix_col = 0;
        self.tna_ribbon_pos = 0;
        self.tna_show_rejected = false;
        self.tna_find = None;
        self.tna_find_editing = false;
        self.tna_sel = 0;
        self.tna_focus_id = None;
        self.tna_detail_scroll = 0;
        self.tna_ledger_sel = 0;
        self.prompt.clear();
        self.cursor = 0;
        self.transcripts.remove(&format!("report:{id}"));
        if let Some(path) = self.tna_db_path.clone() {
            self.tna_source.clear();
            self.tna_source_error = None;
            let tx = self.tx.clone();
            let report_id = report.id.clone();
            let generation = self.report_source_generation;
            argos_osint_core::workers::spawn_blocking(move || {
                let result = Store::open(&path)
                    .and_then(|store| {
                        store.sync_report_index()?;
                        store
                            .report_version(&report_id, None)?
                            .ok_or_else(|| anyhow::anyhow!("Report text unavailable"))
                    })
                    .map_err(|e| e.to_string());
                let _ = tx.send(AppMsg::ReportSource {
                    report_id,
                    generation,
                    version: None,
                    result,
                });
            });
        } else {
            match std::fs::read_to_string(&report.path) {
                Ok(text) => {
                    self.tna_source = text;
                    self.tna_source_error = None;
                }
                Err(err) => {
                    self.tna_source.clear();
                    self.tna_source_error = Some(format!("Report evidence unavailable: {err}"));
                }
            }
        }
        self.status = "ready".into();
        self.ensure_tna_snapshot(false);
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
        self.retrieve_question(text);
    }

    fn retrieve_question(&mut self, query: String) {
        if self.running {
            self.status = "busy".into();
            return;
        }
        self.turn_generation = self.turn_generation.wrapping_add(1);
        let generation = self.turn_generation;
        let scope = if let Some(id) = &self.chat_report {
            match &self.evidence_scope {
                argos_osint_core::evidence::EvidenceScope::Reports(_)
                | argos_osint_core::evidence::EvidenceScope::Case(_)
                | argos_osint_core::evidence::EvidenceScope::Collection => {
                    self.evidence_scope.clone()
                }
                _ => argos_osint_core::evidence::EvidenceScope::Report(id.clone()),
            }
        } else {
            self.evidence_scope.clone()
        };
        self.cancel = Arc::new(AtomicBool::new(false));
        self.running = true;
        self.status = "searching reviewed evidence and report passages".into();
        if let Some(path) = self.tna_db_path.clone() {
            let tx = self.tx.clone();
            argos_osint_core::workers::spawn_blocking(move || {
                let result = Store::open(&path)
                    .and_then(|store| {
                        store.sync_report_index()?;
                        Ok((
                            store.retrieve_passages(&query, &scope, 8)?,
                            store.retrieve_reviewed(&query, &scope, 8)?,
                        ))
                    })
                    .map_err(|e| e.to_string());
                let _ = tx.send(AppMsg::ReviewedRetrieved {
                    generation,
                    query,
                    result,
                });
            });
        } else {
            let result = self
                .store
                .sync_report_index()
                .and_then(|_| {
                    Ok((
                        self.store.retrieve_passages(&query, &scope, 8)?,
                        self.store.retrieve_reviewed(&query, &scope, 8)?,
                    ))
                })
                .map_err(|e| e.to_string());
            self.on_msg(AppMsg::ReviewedRetrieved {
                generation,
                query,
                result,
            });
        }
    }

    fn answer_retrieval(
        &mut self,
        generation: u64,
        query: String,
        result: Result<Vec<argos_osint_core::evidence::PassageHit>, String>,
    ) {
        if generation != self.turn_generation {
            return;
        }
        self.running = false;
        let hits = match result {
            Ok(hits) => hits,
            Err(err) => {
                self.status = format!("Passage retrieval failed: {err}");
                return;
            }
        };
        self.recommendations = hits;
        let mut material = argos_osint_core::evidence::answer_material(&self.recommendations);
        if !self.reviewed_recommendations.is_empty() {
            material.push_str("\nREVIEWED CASE EVIDENCE. Acceptance is analyst review, not independent verification. Cite observation IDs, preserve attribution.\n");
            for f in &self.reviewed_recommendations {
                let o = &f.observation;
                material.push_str(&format!(
                    "[{}] {} · {} · event {} · retrieved {}\n{}\n",
                    o.id,
                    o.case_id.as_deref().unwrap_or("unassigned"),
                    o.attribution,
                    o.event_time.as_deref().unwrap_or("undated"),
                    o.retrieved_at,
                    o.statement
                ));
            }
        }
        if brain::insists_on_new_case(&query) && self.chat_report.is_none() {
            self.desk_memory_answer = Some(material);
            self.push_line("user", &query);
            self.open_scope(query, false, false);
        } else {
            let mut answer = String::new();
            if self.recommendations.is_empty() && self.reviewed_recommendations.is_empty() {
                answer.push_str("No supporting reviewed evidence in this scope.\n");
            } else {
                answer.push_str("Supported saved material (attributed excerpts):\n");
            }
            for (i, p) in self.recommendations.iter().enumerate() {
                answer.push_str(&format!(
                    "\n[{}] {} · {}\n{}\nRecommended passage: /cite {}\n",
                    p.citation(),
                    p.title,
                    p.created_at,
                    p.text,
                    i + 1
                ));
            }
            for f in &self.reviewed_recommendations {
                let o = &f.observation;
                answer.push_str(&format!("\n[{}] {}\nAttribution: {} · reviewed · event {} · retrieved {}\nRecommended entity: {} · case /case {} · source /source {}\n",o.id,o.statement,o.attribution,o.event_time.as_deref().unwrap_or("undated"),o.retrieved_at,o.entity_id,o.case_id.as_deref().unwrap_or("unassigned"),o.id));
            }
            answer.push_str(&format!("\nUnresolved: retrieval does not establish completeness or answer unsupported parts of the question.\nSuggested investigation: collect public sources addressing ‘{query}’. Open its scope with /new {query}. No research or report has started."));
            if let Some(w) = self.investigation.as_mut() {
                w.source = Some(answer);
                w.scroll = 0;
            } else if self.chat_report.is_some() {
                self.tna_answer = Some(TnaAnswer {
                    question: query,
                    answer,
                    ..Default::default()
                });
                self.tna_answer_scroll = 0;
            } else {
                self.push_line("user", &query);
                self.push_line("assistant", &answer);
            }
            self.status = "Saved evidence · explicit investigation available".into();
        }
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
            self.spawn_answered_turn(
                query,
                argos_osint_core::evidence::answer_material(&[]),
                false,
                true,
                false,
            );
        }
    }

    pub fn research_name(&self) -> String {
        self.research_phase
            .providers()
            .get(self.research_sel)
            .unwrap_or(&"search")
            .to_string()
    }
    fn load_research_fields(&mut self) {
        if self.research_phase == argos_osint_core::research::ResearchPhase::Analysis {
            self.fields = vec![
                field(
                    "analysis_report_mode",
                    "Default report kind: final/addendum/revision/followup",
                    self.settings.analysis.report_mode.clone(),
                    false,
                ),
                field(
                    "analysis_lead_limit",
                    "Visible lead limit (1–10)",
                    self.settings.analysis.lead_limit.to_string(),
                    false,
                ),
                field(
                    "__analysis_save",
                    "Save output preferences",
                    "enter".into(),
                    false,
                ),
            ];
            return;
        }
        let name = self.research_name();
        let c = self
            .settings
            .research
            .get(&name)
            .cloned()
            .unwrap_or_default();
        self.fields = vec![
            field(
                "__research_next",
                "Integration",
                format!("{name} · select next"),
                false,
            ),
            field(
                "research_enabled",
                "Enabled (true/false)",
                c.enabled.to_string(),
                false,
            ),
            field("research_endpoint", "Endpoint", c.endpoint.clone(), false),
            field(
                "research_secret",
                "Credential (masked)",
                self.auth
                    .research
                    .get(&c.secret_ref)
                    .cloned()
                    .unwrap_or_default(),
                true,
            ),
            field(
                "research_secret_ref",
                "Secret reference",
                c.secret_ref.clone(),
                false,
            ),
            field(
                "research_executable",
                "Executable path",
                c.executable.clone(),
                false,
            ),
            field(
                "research_container",
                "Pinned container",
                c.container.clone(),
                false,
            ),
            field(
                "research_mode",
                "Mode: native/local/container",
                format!("{:?}", c.mode),
                false,
            ),
            field(
                "research_config",
                "Configuration path",
                c.config_path.clone(),
                false,
            ),
            field(
                "research_dataset",
                "Dataset path",
                c.dataset_path.clone(),
                false,
            ),
            field(
                "research_sites",
                "Selected sites (comma separated)",
                c.selected_sites.join(","),
                false,
            ),
            field(
                "research_dataset_version",
                "Dataset version",
                c.dataset_version.clone(),
                false,
            ),
            field(
                "research_refresh",
                "Dataset refresh days (manual)",
                c.refresh_days.to_string(),
                false,
            ),
            field(
                "research_modules",
                "Selected modules (comma separated)",
                c.selected_modules.join(","),
                false,
            ),
            field(
                "research_hosts",
                "Allowed hosts (comma separated)",
                c.allowed_hosts.join(","),
                false,
            ),
            field(
                "research_timeout",
                "Timeout seconds",
                c.timeout_secs.to_string(),
                false,
            ),
            field(
                "research_concurrency",
                "Concurrency",
                c.concurrency.to_string(),
                false,
            ),
            field(
                "research_rate",
                "Request interval ms",
                c.rate_interval_ms.to_string(),
                false,
            ),
            field(
                "research_results",
                "Result limit",
                c.result_limit.to_string(),
                false,
            ),
            field(
                "research_cache",
                "Cache freshness seconds",
                c.cache_secs.to_string(),
                false,
            ),
            field(
                "research_retries",
                "Retry limit",
                c.retries.to_string(),
                false,
            ),
            field(
                "research_depth",
                "Pivot/crawl depth",
                c.depth.to_string(),
                false,
            ),
            field(
                "research_sensitive",
                "Allow exposure lookups (true/false)",
                c.allow_sensitive.to_string(),
                false,
            ),
            field(
                "research_duration",
                "Collection duration seconds",
                c.duration_secs.to_string(),
                false,
            ),
            field(
                "research_profile",
                "Scan profile",
                c.scan_profile.clone(),
                false,
            ),
            field(
                "research_verified_domains",
                "Verified domains (monitoring unavailable)",
                c.verified_domains.join(","),
                false,
            ),
            field(
                "research_active",
                "Allow active crawling (true/false)",
                c.allow_active.to_string(),
                false,
            ),
            field(
                "__research_save",
                "Save Research Configuration",
                "enter".into(),
                false,
            ),
            field(
                "__research_test",
                "Test Configuration (offline)",
                "enter".into(),
                false,
            ),
            field(
                "__research_install",
                "Install · show plan",
                "enter".into(),
                false,
            ),
            field(
                "__research_update",
                "Update · show plan",
                "enter".into(),
                false,
            ),
            field(
                "__research_verify",
                "Verify installation",
                "enter".into(),
                false,
            ),
            field(
                "__research_remove",
                "Remove Argos-managed installation",
                "enter".into(),
                false,
            ),
        ];
        if self.tool_plan.is_some() {
            self.fields.push(field(
                "__research_apply",
                "Apply the displayed install/update plan",
                "enter".into(),
                false,
            ));
        }
        self.field_sel = self.field_sel.min(self.fields.len().saturating_sub(1));
    }
    fn research_field_action(&mut self, key: &str) {
        use argos_osint_core::research::{test_configuration, ExecutionMode};
        if key == "__analysis_save" {
            let mode = self.field_value("analysis_report_mode");
            if !["final", "addendum", "revision", "followup"].contains(&mode.as_str()) {
                self.status = "Choose final/addendum/revision/followup".into();
                return;
            }
            self.settings.analysis.report_mode = mode;
            self.settings.analysis.lead_limit = self
                .field_value("analysis_lead_limit")
                .parse::<usize>()
                .unwrap_or(5)
                .clamp(1, 10);
            self.status = match self.save_settings_config(&self.settings) {
                Ok(()) => "Output preferences saved".into(),
                Err(e) => e.to_string(),
            };
            return;
        }
        if key == "__research_next" {
            self.research_sel =
                (self.research_sel + 1) % self.research_phase.providers().len().max(1);
            self.load_research_fields();
            return;
        }
        let name = self.research_name();
        let mut c = self
            .settings
            .research
            .get(&name)
            .cloned()
            .unwrap_or_default();
        if key == "__research_save" {
            c.enabled = self.field_value("research_enabled") == "true";
            c.endpoint = self.field_value("research_endpoint");
            c.executable = self.field_value("research_executable");
            c.config_path = self.field_value("research_config");
            c.dataset_path = self.field_value("research_dataset");
            c.dataset_version = self.field_value("research_dataset_version");
            c.refresh_days = self
                .field_value("research_refresh")
                .parse()
                .unwrap_or(30)
                .max(1);
            c.secret_ref = self.field_value("research_secret_ref");
            c.container = self.field_value("research_container");
            c.mode = match self.field_value("research_mode").to_lowercase().as_str() {
                "local" | "localexecutable" => ExecutionMode::LocalExecutable,
                "container" => ExecutionMode::Container,
                _ => ExecutionMode::NativeHttp,
            };
            c.selected_sites = self
                .field_value("research_sites")
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect();
            c.selected_modules = self
                .field_value("research_modules")
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect();
            c.allowed_hosts = self
                .field_value("research_hosts")
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect();
            c.timeout_secs = self
                .field_value("research_timeout")
                .parse()
                .unwrap_or(30)
                .clamp(1, 600);
            c.concurrency = self
                .field_value("research_concurrency")
                .parse()
                .unwrap_or(1)
                .clamp(1, 4);
            c.rate_interval_ms = self
                .field_value("research_rate")
                .parse()
                .unwrap_or(1000)
                .clamp(100, 3600000);
            c.result_limit = self
                .field_value("research_results")
                .parse()
                .unwrap_or(20)
                .clamp(1, 200);
            c.cache_secs = self.field_value("research_cache").parse().unwrap_or(86400);
            c.retries = self
                .field_value("research_retries")
                .parse()
                .unwrap_or(1)
                .min(3);
            c.depth = self
                .field_value("research_depth")
                .parse()
                .unwrap_or(1)
                .min(3);
            c.duration_secs = self
                .field_value("research_duration")
                .parse()
                .unwrap_or(30)
                .clamp(1, 600);
            c.scan_profile = self.field_value("research_profile");
            c.verified_domains = self
                .field_value("research_verified_domains")
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect();
            c.allow_sensitive = self.field_value("research_sensitive") == "true";
            c.allow_active = self.field_value("research_active") == "true";
            let secret = self.field_value("research_secret");
            if !secret.is_empty() && c.secret_ref.is_empty() {
                self.status = "Set a secret reference before saving a credential".into();
                return;
            }
            if !c.secret_ref.is_empty() {
                if secret.is_empty() {
                    self.auth.research.remove(&c.secret_ref);
                } else {
                    self.auth.research.insert(c.secret_ref.clone(), secret);
                }
            }
            if let Err(e) = self.save_auth_config(&self.auth) {
                self.status = format!("Credential save failed: {e}");
                return;
            }
            self.settings.research.insert(name, c);
            self.status = match self.save_settings_config(&self.settings) {
                Ok(()) => "Research configuration saved".into(),
                Err(e) => format!("Configuration save failed: {e}"),
            };
            return;
        }
        if matches!(key, "__research_test" | "__research_verify") {
            let has_secret = self.auth.research.contains_key(&c.secret_ref);
            let tx = self.tx.clone();
            tokio::spawn(async move {
                let (state, detail) = test_configuration(&name, &c, has_secret).await;
                let _ = tx.send(AppMsg::ResearchTest {
                    name,
                    state,
                    detail,
                });
            });
            self.status = "Testing configuration in background".into();
            return;
        }
        if key == "__research_apply" {
            self.apply_research_tool_plan();
            return;
        }
        if matches!(
            key,
            "__research_install" | "__research_update" | "__research_remove"
        ) {
            self.manage_research_tool(&name, key);
            return;
        }
    }
    fn load_coverage(&mut self) {
        let Some(path) = self.tna_db_path.clone() else {
            return;
        };
        let scope = self.evidence_scope.clone();
        let report_id = self.chat_report.clone();
        let tx = self.tx.clone();
        let themes = self
            .tna_snapshot()
            .map(|s| {
                s.nodes
                    .iter()
                    .filter(|n| n.kind == TnaNodeKind::Topic)
                    .map(|n| n.label.clone())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        argos_osint_core::workers::spawn_blocking(move || {
            let result = Store::open(&path)
                .and_then(|s| {
                    s.sync_report_index()?;
                    s.coverage(&scope, &themes)
                })
                .map_err(|e| e.to_string());
            let _ = tx.send(AppMsg::Coverage { report_id, result });
        });
    }
    fn open_coverage_passage(&mut self) {
        let Some(path) = self.tna_db_path.clone() else {
            return;
        };
        let Some(row) = self.coverage_rows.get(self.tna_matrix_row) else {
            return;
        };
        let Some(cell) = row.cells.get(self.tna_matrix_col) else {
            return;
        };
        let query = row.theme.clone();
        let scope = argos_osint_core::evidence::EvidenceScope::Report(cell.report_id.clone());
        let tx = self.tx.clone();
        argos_osint_core::workers::spawn_blocking(move || {
            let result = Store::open(&path)
                .and_then(|s| Ok(s.retrieve_passages(&query, &scope, 1)?.into_iter().next()))
                .map_err(|e| e.to_string());
            let _ = tx.send(AppMsg::OpenPassage(result));
        });
    }
    pub fn entity_coverage(&self, entity_id: &str) -> String {
        let jobs = self
            .research_jobs
            .iter()
            .filter(|j| j.input.entity_id == entity_id && j.input.report_id == self.chat_report)
            .collect::<Vec<_>>();
        if jobs.is_empty() {
            return "uninvestigated".into();
        }
        if jobs.iter().any(|j| {
            matches!(
                j.state,
                argos_osint_core::research::JobState::Running
                    | argos_osint_core::research::JobState::Queued
            )
        }) {
            return "partial".into();
        }
        if jobs.iter().all(|j| {
            matches!(
                j.state,
                argos_osint_core::research::JobState::Failed
                    | argos_osint_core::research::JobState::RateLimited
            )
        }) {
            return "blocked".into();
        }
        if let Some(job) = jobs.first() {
            let freshness = self
                .settings
                .research
                .get(&job.provider)
                .map(|c| c.cache_secs)
                .unwrap_or(86400);
            if job
                .finished_at
                .as_ref()
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                .is_some_and(|t| {
                    chrono::Utc::now().signed_duration_since(t).num_seconds() > freshness as i64
                })
            {
                return "stale".into();
            }
        }
        if jobs
            .iter()
            .all(|j| j.state == argos_osint_core::research::JobState::Completed)
        {
            "fresh".into()
        } else {
            "partial".into()
        }
    }
    fn manage_research_tool(&mut self, name: &str, action: &str) {
        let root = paths::home_dir().join("tools");
        if action == "__research_remove" {
            let c = self
                .settings
                .research
                .get(name)
                .cloned()
                .unwrap_or_default();
            if c.executable.is_empty() {
                self.status = "No managed installation to remove".into();
                return;
            }
            self.tool_plan = Some((
                name.into(),
                action.into(),
                argos_osint_core::tool_manager::InstallPlan {
                    tool: name.into(),
                    version: c.detected_version,
                    source: "Remove this Argos-managed installation only".into(),
                    destination: PathBuf::from(c.executable),
                    prerequisites: vec![
                        "Managed manifest and exact binary required; other installations retained"
                            .into(),
                    ],
                    archive: String::new(),
                    checksums: String::new(),
                },
            ));
            self.load_research_fields();
            self.field_sel = self.fields.len().saturating_sub(1);
            self.status = "Review removal destination; Apply starts the background job".into();
            return;
        }
        match argos_osint_core::tool_manager::plan(name, &root) {
            Ok(plan) => {
                self.tool_plan = Some((name.into(), action.into(), plan));
                self.load_research_fields();
                self.field_sel = self.fields.len().saturating_sub(1);
                self.status =
                    "Review the installation plan; Apply starts the visible background job".into();
            }
            Err(err) => {
                self.status = err.to_string();
                self.log_event("system", &format!("{name} setup: {err}"));
            }
        }
    }
    fn apply_research_tool_plan(&mut self) {
        let Some(queue) = self.research_queue.clone() else {
            self.status = "File-backed research queue required".into();
            return;
        };
        let Some((name, action, plan)) = self.tool_plan.take() else {
            return;
        };
        let tx = self.tx.clone();
        self.research_active += 1;
        self.status = format!(
            "Installing {} {} in background · keyboard remains available",
            plan.tool, plan.version
        );
        if let Some(c) = self.settings.research.get_mut(&name) {
            c.readiness = argos_osint_core::research::Readiness::Installing;
        }
        tokio::spawn(async move {
            let removing = action == "__research_remove";
            let remove_path = if removing {
                Some(plan.destination.clone())
            } else {
                None
            };
            let (_job, result) = queue
                .manage_tool(
                    &name,
                    if removing {
                        "remove"
                    } else if action == "__research_update" {
                        "update"
                    } else {
                        "install"
                    },
                    if removing { None } else { Some(plan) },
                    remove_path,
                )
                .await;
            let _ = tx.send(AppMsg::ToolManaged { name, result });
        });
    }
    fn submit_enrichment(&mut self, arg: &str) {
        self.submit_enrichment_with_plan(arg, self.settings.source_plan());
    }
    fn submit_enrichment_with_plan(&mut self, arg: &str, plan: SourcePlan) {
        if self.case_data_busy.is_some() {
            self.status = "Wait for case data removal to finish".into();
            return;
        }
        if self.chat_case.is_none() && self.chat_report.is_none() {
            self.status = "Create or open a case first: + or /case <id>".into();
            return;
        }
        if self.research_active >= 8 {
            self.status = "Research queue limit reached; wait for a job or cancel".into();
            return;
        }
        let mut parts = arg.splitn(2, ' ');
        let provider = parts.next().unwrap_or("");
        let typed = parts.next().unwrap_or("").trim();
        let case_lead = self.investigation.as_ref().and_then(|w| w.lead()).cloned();
        let selected = self.tna_focus_node().cloned();
        let label = if typed.is_empty() {
            case_lead
                .as_ref()
                .map(|e| e.label.clone())
                .or_else(|| selected.as_ref().map(|n| n.label.clone()))
                .unwrap_or_else(|| {
                    self.investigation
                        .as_ref()
                        .map(|w| w.question.clone())
                        .unwrap_or_default()
                })
        } else {
            typed.to_string()
        };
        let Some(config) = self.settings.research.get(provider).cloned() else {
            self.status="/investigate <integration> [selected entity or focused query] · i opens cached evidence and actions".into();
            return;
        };
        let Some(queue) = self.research_queue.clone() else {
            self.status = "Research queue requires a file-backed workspace".into();
            return;
        };
        if self
            .investigation
            .as_ref()
            .and_then(|w| w.data.scope.as_ref())
            .is_some_and(|s| !s.allowed_actions.contains(&provider.to_string()))
        {
            self.status = "Action outside investigation scope".into();
            return;
        }
        if provider == "shodan"
            && self
                .auth
                .research
                .get(&config.secret_ref)
                .is_none_or(|s| s.is_empty())
        {
            self.status = "Shodan credential unavailable; configure Providers → Research".into();
            return;
        }
        if let Err(e) = argos_osint_core::research::eligible_action(provider, &label, &config) {
            self.status = e.to_string();
            return;
        }
        if self.investigation.as_ref().is_some_and(|w| {
            w.data.gaps.iter().any(|g| {
                g.kind == argos_osint_core::investigation::GapKind::CollectedAbsent
                    && g.action.as_deref() == Some(provider)
                    && g.input == label
            })
        }) {
            self.status="collected_absent · not a real-world negative finding · change scope/input before another collection".into();
            return;
        }
        if let Some(w) = self.investigation.as_mut() {
            w.switch_view(CaseView::Review);
        }
        let entity_id = if typed.is_empty() {
            case_lead
                .map(|e| e.id)
                .or_else(|| selected.map(|n| n.id))
                .unwrap_or_else(|| label.clone())
        } else {
            let parsed = argos_osint_core::search::TextQuery::extract(&label);
            let kind = if label.parse::<std::net::IpAddr>().is_ok() {
                argos_osint_core::evidence::EntityType::Ip
            } else if parsed.emails == vec![label.clone()] {
                argos_osint_core::evidence::EntityType::Email
            } else if parsed.domains == vec![label.clone()] {
                argos_osint_core::evidence::EntityType::Domain
            } else {
                argos_osint_core::evidence::EntityType::Theme
            };
            let e = argos_osint_core::investigation::normalize_entity(&label, kind).ok();
            if let Some(e) = &e {
                let _ = self.store.put_record(
                    &e.id,
                    "entity",
                    self.chat_case.as_deref(),
                    self.chat_report.as_deref(),
                    e,
                );
            }
            e.map(|e| e.id).unwrap_or_else(|| label.clone())
        };
        let input = argos_osint_core::research::ResearchInput {
            case_id: self
                .open_report()
                .and_then(|r| r.case_id.clone())
                .or_else(|| self.chat_case.clone()),
            report_id: self.chat_report.clone(),
            entity_id,
            label,
            action: provider.into(),
            depth: 0,
        };
        let secret = self.auth.research.get(&config.secret_ref).cloned();
        let provider = provider.to_string();
        let tx = self.tx.clone();
        self.research_active += 1;
        if let Some(id) = &input.case_id {
            *self.case_pending_work.entry(id.clone()).or_default() += 1;
        }
        self.status = "Research queued · /jobs · /cancel-jobs".into();
        tokio::spawn(async move {
            let job = queue.execute(input, &provider, config, plan, secret).await;
            let _ = tx.send(AppMsg::ResearchJob(job));
        });
    }
    fn show_evidence_surface(&mut self, action: &str, arg: &str) {
        if self.case_data_busy.is_some() {
            self.status = "Wait for case data removal to finish".into();
            return;
        }
        let Some(path) = self.tna_db_path.clone() else {
            self.status = "Evidence review requires a file-backed workspace".into();
            return;
        };
        let report_id = self.chat_report.clone();
        let evidence_scope = self.evidence_scope.clone();
        let entity = self.tna_focus_node().cloned();
        let action = action.to_string();
        let arg = arg.to_string();
        let tx = self.tx.clone();
        let title = action.clone();
        argos_osint_core::workers::spawn_blocking(move || {
            let result=(||->Result<String>{
                use argos_osint_core::evidence::{Correction,ReviewDecision};
                let store=Store::open(&path)?;
                if action=="review" {let mut parts=arg.splitn(3,' ');let id=parts.next().unwrap_or("");if !store.findings_scoped(&evidence_scope)?.iter().any(|f|f.observation.id==id){anyhow::bail!("Observation is not in the selected evidence scope");}let decision=match parts.next().unwrap_or(""){"retain"=>ReviewDecision::Retain,"accept"=>ReviewDecision::Accept,"reject"=>ReviewDecision::Reject,"defer"=>ReviewDecision::Defer,_=>anyhow::bail!("/review <observation id> retain|accept|reject|defer <reason>")};let reason=parts.next().unwrap_or("").trim();if reason.is_empty(){anyhow::bail!("Add a review reason");}store.review_finding(id,decision,reason)?;return Ok("Review decision saved with history. /findings to inspect; /save-update addendum|revision|followup".into());}
                if let argos_osint_core::evidence::EvidenceScope::Case(case_id)=&evidence_scope {
                    if action=="merge"||action=="unmerge" {
                        let (ids,reason)=arg.split_once(" --reason ").unwrap_or((&arg,""));
                        let entities=ids.split_whitespace().map(str::to_string).collect::<Vec<_>>();
                        if reason.trim().is_empty(){anyhow::bail!("/merge <canonical> <other IDs> --reason <reason>; /unmerge <id> --reason <reason>");}
                        let evidence=store.findings_scoped(&evidence_scope)?.into_iter().filter(|f|f.decision==Some(ReviewDecision::Accept)&&entities.contains(&f.observation.entity_id)).flat_map(|f|f.observation.evidence).collect();
                        let decision=argos_osint_core::evidence::IdentityDecision{id:argos_osint_core::store::new_id("identity-decision"),canonical_entity:if action=="merge"{entities.first().cloned()}else{None},reverses:if action=="unmerge"{entities.first().cloned()}else{None},entities,reason:reason.into(),evidence};
                        store.save_case_identity(case_id,&decision)?;return Ok(format!("Identity decision {} saved. /unmerge {} --reason <reason> reverses it.",decision.id,decision.id));
                    }
                    if action=="correct"||action=="undo-correction" {
                        use argos_osint_core::investigation::EntityCorrection;
                        let correction=if action=="undo-correction"{EntityCorrection{id:argos_osint_core::store::new_id("entity-correction"),entity_id:String::new(),original:String::new(),replacement:None,reason:"Explicit analyst reversal".into(),reverses:Some(arg.clone())}}
                        else {let mut words=arg.splitn(3,' ');let id=words.next().unwrap_or("");let replacement=words.next().unwrap_or("suppress");let reason=words.next().unwrap_or("");let entity=store.records_scoped::<argos_osint_core::evidence::Entity>("entity",&evidence_scope)?.into_iter().find(|e|e.id==id).ok_or_else(||anyhow::anyhow!("Entity not found in case"))?;EntityCorrection{id:argos_osint_core::store::new_id("entity-correction"),entity_id:id.into(),original:entity.label,replacement:if replacement=="suppress"{None}else{Some(replacement.replace('_'," "))},reason:reason.into(),reverses:None}};
                        store.correct_case_entity(case_id,&correction)?;return Ok(format!("Correction {} saved. /undo-correction {} reverses it.",correction.id,correction.id));
                    }
                }
                if action=="merge" || action=="unmerge" {
                    let report=report_id.clone().ok_or_else(||anyhow::anyhow!("Open a report first"))?;
                    let tokens=arg.split_whitespace().map(str::to_string).collect::<Vec<_>>();
                    let decision=argos_osint_core::evidence::IdentityDecision{id:argos_osint_core::store::new_id("identity-decision"),entities:if action=="merge"{tokens.clone()}else{vec![]},canonical_entity:if action=="merge"{tokens.first().cloned()}else{None},reason:"Explicit analyst identity resolution; original identifiers and mentions retained".into(),evidence:vec![],reverses:if action=="unmerge"{tokens.first().cloned()}else{None}};
                    store.save_identity_decision(&report,&decision)?;tna::rebuild_for_report(&store,&report)?;
                    return Ok(format!("Identity decision {} saved. /unmerge {} reverses it. Reopen network to refresh.",decision.id,decision.id));
                }
                if action=="correct" || action=="undo-correction" {
                    let id=report_id.clone().ok_or_else(||anyhow::anyhow!("Open a report first"))?;
                    let correction=if action=="undo-correction" {Correction{id:argos_osint_core::store::new_id("correction"),report_id:id,start:0,original:String::new(),replacement:None,entity_type:None,reason:"Analyst reversal".into(),reverses:Some(arg.clone())}}
                    else {let mut parts=arg.splitn(3,' ');let start=parts.next().unwrap_or("").parse::<usize>()?;let change=parts.next().unwrap_or("suppress");let reason=parts.next().unwrap_or("Analyst correction");let snap=tna::rebuild_for_report(&store,&id)?;let d=snap.decisions.iter().find(|d|d.start==start).ok_or_else(||anyhow::anyhow!("Source mention offset not found"))?;Correction{id:argos_osint_core::store::new_id("correction"),report_id:id,start,original:d.original.clone(),replacement:if change=="suppress"{None}else{Some(change.replace('_'," "))},entity_type:Some(d.kind),reason:reason.into(),reverses:None}};
                    store.save_correction(&correction)?;tna::rebuild_for_report(&store,&correction.report_id)?;
                    return Ok(format!("Correction {} saved. /undo-correction {} reverses it. Reopen the network to refresh.",correction.id,correction.id));
                }
                if action=="timeline" {store.sync_report_index()?;return Ok(store.timeline_scoped(&evidence_scope)?.iter().map(|e|format!("{} · event {} · published {} · retrieved {}\n{}\n{}\n{}",e.lane,e.event_time.as_deref().unwrap_or("undated"),e.published_at.as_deref().unwrap_or("unknown"),if e.retrieved_at.is_empty(){"unknown"}else{&e.retrieved_at},e.statement,e.uncertainty.as_deref().unwrap_or(""),e.evidence.iter().filter_map(|r|r.passage_id.as_ref()).filter_map(|id|store.passage(id).ok().flatten()).map(|p|format!("[{}]",p.citation())).collect::<Vec<_>>().join(" "))).collect::<Vec<_>>().join("\n\n"));}
                let findings=store.findings_scoped(&evidence_scope)?;
                let mut out=if action=="inspect" {format!("Selected: {}\nCached evidence; cursor movement never collects.\n\nNext actions: /investigate search|domain|internetdb|leakcheck|shodan|xposedornot|whatsmyname [input]\nCollections require configured tools and privacy/scope opt-ins.\n\n",entity.as_ref().map(|n|n.label.as_str()).unwrap_or("none"))}else{"New observations are separate from conclusions. /review <id> retain|accept|reject|defer <reason>\n/save-update addendum|revision|followup\n\n".into()};
                for f in findings.iter().filter(|f|action!="inspect" || entity.as_ref().is_none_or(|e|e.id==f.observation.entity_id)) {out.push_str(&format!("{} · {:?} · {} · retrieved {}\n{}\n{}\nAttribution: {}\nSources: {}\n\n",f.observation.id,f.decision,f.observation.provider,f.observation.retrieved_at,f.category,f.observation.statement,f.observation.attribution,f.observation.evidence.iter().filter_map(|e|e.source_url.clone()).collect::<Vec<_>>().join(", ")));}
                let relationships=store.records_scoped::<argos_osint_core::evidence::Relationship>("relationship",&evidence_scope)?;
                for relationship in relationships.iter().filter(|r|action!="inspect" || entity.as_ref().is_none_or(|e|e.id==r.from || e.id==r.to)) {out.push_str(&format!("Relationship: {} → {} · {:?} · {:?}\n{}\n\n",relationship.from,relationship.to,relationship.kind,relationship.basis,relationship.uncertainty));}
                if findings.is_empty(){out.push_str("No collected observations in this scope. Evidence is uncollected, not a negative finding.");}
                Ok(out)
            })().map_err(|e|e.to_string());
            let _ = tx.send(AppMsg::EvidenceSurface {
                report_id,
                title,
                result,
                scope: evidence_scope,
            });
        });
    }
    fn save_findings_update(&mut self, arg: &str) {
        use argos_osint_core::evidence::ReportUpdateMode;
        let mode = match arg {
            "addendum" => ReportUpdateMode::Addendum,
            "revision" => ReportUpdateMode::Revision,
            "followup" => ReportUpdateMode::FollowUp,
            _ => {
                self.status = "/save-update addendum|revision|followup".into();
                return;
            }
        };
        let (Some(path), Some(id)) = (self.tna_db_path.clone(), self.chat_report.clone()) else {
            self.status = "Open a saved report first".into();
            return;
        };
        let directory = report_dir(&self.settings);
        let tx = self.tx.clone();
        argos_osint_core::workers::spawn_blocking(move || {
            let result = Store::open(&path)
                .and_then(|store| store.save_report_update(&id, mode, &directory))
                .map_err(|e| e.to_string());
            let _ = tx.send(AppMsg::ReportUpdated(result));
        });
        self.status = "Saving reviewed report update in background".into();
    }
    fn run_slash(&mut self, line: &str) {
        let rest = line.trim_start_matches('/').trim();
        let mut parts = rest.splitn(2, char::is_whitespace);
        let cmd = parts.next().unwrap_or("").to_lowercase();
        let arg = parts.next().unwrap_or("").trim().to_string();
        match cmd.as_str() {
            "case-data" => {
                self.show_case_data_controls();
                return;
            }
            "clear-case" | "delete-case" => {
                self.prepare_case_data(&arg, cmd == "delete-case");
                return;
            }
            "confirm-case" => {
                self.confirm_case_data(&arg);
                return;
            }
            "cancel-case-data" => {
                self.cancel_case_data_plan();
                return;
            }
            _ => {}
        }
        if matches!(cmd.as_str(), "work" | "graph" | "gaps") {
            if let Some(w) = self.investigation.as_mut() {
                if cmd == "work" {
                    if !arg.is_empty() {
                        if let Some(e) = w
                            .data
                            .entities
                            .iter()
                            .find(|e| e.id == arg || e.label.eq_ignore_ascii_case(&arg))
                        {
                            w.lead_id = Some(e.id.clone());
                        }
                    }
                    w.switch_view(CaseView::Review);
                    w.plan_focus = false;
                    w.inbox_focus = false;
                } else {
                    w.switch_view(CaseView::Focus);
                    w.gap_focus = cmd == "gaps";
                    w.gaps_case_wide = arg == "case";
                    w.gap_sel = 0;
                }
                self.focus = Focus::Graph;
            } else if cmd == "gaps" {
                self.desk_pane = 2;
                self.desk_row = 0;
                self.focus = Focus::Canvas;
            } else {
                self.status = "Open a case first: /case <id>".into();
            }
            return;
        }
        if cmd == "source" {
            self.open_observation_source(&arg);
            return;
        }
        if cmd == "case" {
            if arg.is_empty() {
                self.push_line(
                    "assistant",
                    &self
                        .cases
                        .iter()
                        .map(|c| format!("{} · {}", c.id, c.title))
                        .collect::<Vec<_>>()
                        .join("\n"),
                );
            } else if let Some(case) = session::resolve_case(&self.cases, &arg) {
                let id = case.id.clone();
                self.open_investigation(&id);
            } else {
                self.status = "No single matching case".into();
            }
            return;
        }
        if cmd == "filter" {
            if let Some(w) = self.investigation.as_mut() {
                w.filter = arg;
                w.row = 0;
            }
            return;
        }
        if cmd == "draft" {
            self.draft_investigation(&arg);
            return;
        }

        if self.chat_report.is_some()
            && !matches!(
                cmd.as_str(),
                "help"
                    | "?"
                    | "clear"
                    | "find"
                    | "network"
                    | "read"
                    | "cite"
                    | "scope"
                    | "related"
                    | "entities"
                    | "retain-answer"
                    | "investigate"
                    | "inspect"
                    | "findings"
                    | "review"
                    | "save-update"
                    | "timeline"
                    | "correct"
                    | "undo-correction"
                    | "merge"
                    | "unmerge"
                    | "corroborate"
                    | "weakest"
                    | "jobs"
                    | "cancel-jobs"
                    | "model"
                    | "m"
                    | "models"
                    | "quit"
                    | "exit"
                    | "q"
            )
        {
            self.status = format!("/{cmd} is Desk-only; Esc closes the report workspace");
            return;
        }
        match cmd.as_str() {
            "jobs" => {
                if let Some(w) = self.investigation.as_mut() {
                    w.switch_view(CaseView::Jobs);
                    w.global_jobs = true;
                    w.inbox_focus = false;
                    self.focus = Focus::Graph;
                    return;
                }
                let text = self
                    .research_jobs
                    .iter()
                    .take(20)
                    .map(|j| {
                        format!(
                            "{} · {} · {:?} · {}ms · {}",
                            j.id,
                            j.provider,
                            j.state,
                            j.elapsed_ms,
                            j.error.as_deref().unwrap_or(&j.progress)
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                if self.chat_report.is_some() {
                    self.tna_answer = Some(TnaAnswer {
                        question: "Background research".into(),
                        answer: text,
                        ..Default::default()
                    });
                } else {
                    self.push_line("assistant", &text);
                }
            }
            "cancel-jobs" => {
                if let Some(q) = self.research_queue.take() {
                    q.cancel.store(true, Ordering::Relaxed);
                    self.research_queue = Some(argos_osint_core::research::ResearchQueue::new(
                        paths::db_path(),
                        4,
                    ));
                }
                self.attach_research_progress();
                self.status =
                    "Cancellation requested for queued/running research and installers".into();
            }
            "investigate" => self.submit_enrichment(&arg),
            "inspect" | "findings" | "timeline" => self.show_evidence_surface(&cmd, &arg),
            "review" => self.show_evidence_surface("review", &arg),
            "correct" | "undo-correction" | "merge" | "unmerge" => {
                self.show_evidence_surface(&cmd, &arg)
            }
            "corroborate" | "weakest" => {
                let pair = if cmd == "weakest" {
                    self.tna_paths().get(self.tna_path_sel).and_then(|p| {
                        p.nodes
                            .windows(2)
                            .min_by_key(|pair| self.tna_edge_weight(&pair[0], &pair[1]))
                            .map(|pair| (pair[0].clone(), pair[1].clone()))
                    })
                } else {
                    self.tna_links()
                        .get(self.tna_ledger_sel)
                        .map(|e| (e.from.clone(), e.to.clone()))
                };
                if let Some((a, b)) = pair {
                    self.submit_enrichment(&format!(
                        "search {} {}",
                        self.tna_label(&a),
                        self.tna_label(&b)
                    ));
                } else {
                    self.status = "Select an edge or path hop first".into();
                }
            }
            "save-update" => self.save_findings_update(&arg),
            "scope" => {
                if self.investigation.is_some() {
                    self.status =
                        "Case workspace scope is fixed; Esc to Desk to change retrieval scope"
                            .into();
                    return;
                }
                use argos_osint_core::evidence::EvidenceScope;
                self.evidence_scope = if arg == "desk" {
                    EvidenceScope::Desk
                } else if arg == "collection" {
                    EvidenceScope::Collection
                } else if arg == "report" {
                    self.chat_report
                        .clone()
                        .map(EvidenceScope::Report)
                        .unwrap_or_default()
                } else if let Some(id) = arg.strip_prefix("case ") {
                    EvidenceScope::Case(id.to_string())
                } else if let Some(ids) = arg.strip_prefix("reports ") {
                    EvidenceScope::Reports(ids.split_whitespace().map(str::to_string).collect())
                } else {
                    self.status = "/scope desk|report|case <id>|reports <ids>|collection".into();
                    return;
                };
                self.recommendations.clear();
                self.status = format!("Evidence scope: {:?}", self.evidence_scope);
                if self.chat_report.is_some() {
                    self.tna_report = None;
                    self.ensure_tna_snapshot(true);
                }
            }
            "read" => {
                self.workspace_reading = true;
                self.focus = Focus::Graph;
            }
            "cite" => {
                let selected = arg
                    .parse::<usize>()
                    .ok()
                    .and_then(|n| self.recommendations.get(n.saturating_sub(1)))
                    .cloned()
                    .or_else(|| {
                        self.recommendations
                            .iter()
                            .find(|h| h.citation() == arg || h.id == arg)
                            .cloned()
                    });
                if let Some(hit) = selected {
                    self.open_report_chat(&hit.report_id);
                    self.selected_passage =
                        Some((hit.report_id.clone(), hit.start, hit.end, hit.version));
                    self.report_source_version = Some(hit.version);
                    if let Some(path) = self.tna_db_path.clone() {
                        let tx = self.tx.clone();
                        let report_id = hit.report_id.clone();
                        let generation = self.report_source_generation;
                        let version = hit.version;
                        argos_osint_core::workers::spawn_blocking(move || {
                            let result = Store::open(&path)
                                .and_then(|s| {
                                    s.report_version(&report_id, Some(version))?
                                        .ok_or_else(|| anyhow::anyhow!("Citation version missing"))
                                })
                                .map_err(|e| e.to_string());
                            let _ = tx.send(AppMsg::ReportSource {
                                report_id,
                                generation,
                                version: Some(version),
                                result,
                            });
                        });
                    } else if let Ok(Some(text)) =
                        self.store.report_version(&hit.report_id, Some(hit.version))
                    {
                        self.report_read_source = Some(text);
                    }
                    self.report_read_line = hit.line.saturating_sub(1);
                    self.workspace_reading = true;
                    self.focus_selected_passage();
                    self.status = format!(
                        "Citation [{}] · {} · network uses latest report evidence",
                        hit.citation(),
                        hit.section
                    );
                } else if let Some(path) = self.tna_db_path.clone() {
                    let tx = self.tx.clone();
                    let citation = arg.clone();
                    argos_osint_core::workers::spawn_blocking(move || {
                        let result = Store::open(&path)
                            .and_then(|s| s.passage_citation(&citation))
                            .map_err(|e| e.to_string());
                        let _ = tx.send(AppMsg::OpenPassage(result));
                    });
                } else {
                    self.status = "/cite <recommendation number or report@version:line>".into();
                }
            }
            "related" => {
                let query = self
                    .open_report()
                    .map(|r| r.title.clone())
                    .unwrap_or_default();
                self.retrieve_question(query);
            }
            "entities" => {
                self.workspace_reading = false;
                self.set_tna_layout(TnaLayout::Cockpit);
            }
            "retain-answer" => {
                if let Some(a) = self
                    .tna_answer
                    .as_ref()
                    .filter(|a| !a.pending && a.error.is_none())
                {
                    let text = a.answer.clone();
                    self.capture_report_insight(&text);
                }
            }
            "help" | "?" => self.help = true,
            "quit" | "exit" | "q" => self.quit = true,
            "dashboard" | "home" => {
                if self.investigation.is_some() {
                    self.close_investigation();
                } else {
                    self.on_esc();
                }
                self.case_page = CasePage::Closed;
                self.module = Some(ModuleId::Cases);
                self.chat_report = None;
                self.desk_transcript = false;
            }
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
                    self.open_investigation(&case.id);
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
                self.model_target = ModelTarget::Writer;
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
                if let Some(w) = self.investigation.as_mut() {
                    w.view = case_workspace::CaseView::Focus;
                    self.focus = Focus::Graph;
                    return;
                }
                self.workspace_reading = false;
                self.select_case_page(CasePage::Network);
            }
            "find" => {
                if self.case_page != CasePage::Network {
                    self.select_case_page(CasePage::Network);
                }
                if self.case_page == CasePage::Network {
                    self.tna_find = Some(arg);
                    self.tna_find_editing = true;
                    self.focus = Focus::Graph;
                }
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
                let _ = self.save_settings_config(&self.settings);
                self.push_line(
                    "assistant",
                    "Modality is voice. Ctrl+R records, Enter sends the transcript.",
                );
            }
            "text" => {
                self.settings.modality = "text".into();
                let _ = self.save_settings_config(&self.settings);
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
        if self.chat_report.is_some() {
            self.dismiss_tna_answer();
            return;
        }
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
                let _ = self.store.add_report_metadata(&meta);
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
            if self.chat_report.is_none() {
                self.status = "busy".into();
                return;
            }
            self.cancel.store(true, Ordering::Relaxed);
        }
        if self.chat_report.is_none()
            && prior_reports.is_empty()
            && prompt::classify(&text) == Intent::Remember
        {
            let fact = prompt::remember_text(&text);
            self.push_line("user", &text);
            if let Ok(mem) = self.store.add_memory(&fact) {
                self.memories.insert(0, mem);
                self.push_line("assistant", &format!("Remembered: {fact}"));
            }
            return;
        }
        if self.chat_report.is_some() {
            self.tna_answer = Some(TnaAnswer {
                question: text.clone(),
                pending: true,
                ..Default::default()
            });
            self.tna_answer_scroll = 0;
            if let Some(err) = self.tna_source_error.clone() {
                self.on_tna_turn(TurnEvent::Failed(err));
                return;
            }
        }
        self.turn_generation = self.turn_generation.wrapping_add(1);
        let generation = self.turn_generation;
        let report_id = self.chat_report.clone();
        self.running = true;
        self.cancel = Arc::new(AtomicBool::new(false));
        self.status = "starting".into();
        if self.chat_report.is_none() {
            if echo_user {
                self.push_line("user", &text);
            }
            self.push_line("assistant", "");
        }
        self.log_event("api", &format!("chat {}", self.active_model()));
        let (prior_reports, evidence_only, from_memory, memories) =
            if let Some(report) = self.open_report().cloned() {
                (
                    if prior_reports.is_empty() {
                        self.tna_report_material(&report)
                    } else {
                        prior_reports
                    },
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
            hardware_line: if report_id.is_some() {
                String::new()
            } else {
                self.hardware.one_line()
            },
            modality: self.settings.modality.clone(),
            provider: Some(self.role_secret(true)),
            tool_provider: Some(self.role_secret(false)),
            plan: self.settings.source_plan(),
            report_dir: report_dir(&self.settings),
            case_id: self.case_id(),
            gmail: self.auth.gmail.as_ref().map(GmailConfig::from),
            citation_ids: self.recommendations.iter().map(|h| h.citation()).collect(),
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
                if tx
                    .send(AppMsg::ScopedTurn {
                        generation,
                        report_id: report_id.clone(),
                        event: ev,
                    })
                    .is_err()
                {
                    break;
                }
            }
            let _ = worker.await;
        });
    }

    pub fn open_module(&mut self, module: ModuleId) {
        if self.investigation.is_some() {
            self.close_investigation();
        }
        if self.chat_report.is_some() {
            if module == ModuleId::Brain {
                self.status = "Brain is available after closing the report".into();
                return;
            }
            if matches!(module, ModuleId::Cases | ModuleId::Reports) {
                self.module = Some(ModuleId::Cases);
                self.focus = Focus::Graph;
                return;
            }
            self.close_report_workspace();
        }
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
                self.provider_page = ProviderPage::Models;
            }
            ModuleId::Osint => {
                self.module = Some(ModuleId::Providers);
                self.provider_page = ProviderPage::Osint;
            }
            ModuleId::Providers => {
                self.module = Some(ModuleId::Providers);
                self.provider_page = ProviderPage::Models;
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
        if self.module == Some(ModuleId::Providers) && self.provider_page == ProviderPage::Research
        {
            self.load_research_fields();
            return;
        }
        match self.form_module() {
            Some(module) => self.load_fields(module),
            None => self.fields.clear(),
        }
    }

    fn cycle_group_page(&mut self, delta: isize) {
        if self.chat_report.is_some() {
            self.set_tna_layout(
                TnaLayout::ALL[(self.tna_layout.index() as isize + delta).rem_euclid(5) as usize],
            );
            return;
        }
        match self.module {
            Some(ModuleId::Cases) => {
                let pages = self.case_pages();
                let index = pages
                    .iter()
                    .position(|page| *page == self.case_page)
                    .unwrap_or(0);
                let next = (index as isize + delta).rem_euclid(pages.len() as isize) as usize;
                self.select_case_page(pages[next]);
                return;
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
                        self.auth
                            .research
                            .get("BRAVE_API_KEY")
                            .cloned()
                            .unwrap_or_else(|| self.settings.brave_key.clone()),
                        true,
                    ),
                    field(
                        "tavily_key",
                        "Tavily API key",
                        self.auth
                            .research
                            .get("TAVILY_API_KEY")
                            .cloned()
                            .unwrap_or_else(|| self.settings.tavily_key.clone()),
                        true,
                    ),
                    field(
                        "youtube_key",
                        "YouTube API key",
                        self.auth
                            .research
                            .get("YOUTUBE_API_KEY")
                            .cloned()
                            .unwrap_or_else(|| self.settings.youtube_key.clone()),
                        true,
                    ),
                    field(
                        "github_token",
                        "GitHub token",
                        self.auth
                            .research
                            .get("GITHUB_TOKEN")
                            .cloned()
                            .unwrap_or_else(|| self.settings.github_token.clone()),
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
                if self.provider_page == ProviderPage::Models {
                    self.fields = vec![
                        field(
                            "__writer_provider",
                            "Provider",
                            provider_name(&provider::effective_kind(&self.role_secret(true)))
                                .into(),
                            false,
                        ),
                        field(
                            "__role_writer",
                            "Model",
                            self.role_secret(true).model,
                            false,
                        ),
                        field(
                            "__tool_provider",
                            "Provider",
                            provider_name(&provider::effective_kind(&self.role_secret(false)))
                                .into(),
                            false,
                        ),
                        field("__role_tool", "Model", self.role_secret(false).model, false),
                    ];
                } else if let Some(kind) = self.provider_page.account() {
                    let secret = provider::account_secret(&self.auth, kind);
                    self.fields = Vec::new();
                    if kind == "grok" {
                        self.fields.push(field(
                            "__grok_subscription_login",
                            "Sign in with Grok",
                            "Enter".into(),
                            false,
                        ));
                        self.fields.push(field(
                            "__grok_subscription_check",
                            "Check existing login",
                            "Enter".into(),
                            false,
                        ));
                        self.fields.push(field(
                            "__models",
                            "Choose Writer / Tools",
                            "Models".into(),
                            false,
                        ));
                    } else if kind == "openai-chatgpt" {
                        self.fields.push(field(
                            "__subscription_login",
                            "Sign in with ChatGPT",
                            "Enter".into(),
                            false,
                        ));
                        self.fields.push(field(
                            "__subscription_check",
                            "Check existing login",
                            "Enter".into(),
                            false,
                        ));
                        self.fields.push(field(
                            "__models",
                            "Choose Writer model",
                            "Models → Writer".into(),
                            false,
                        ));
                    } else {
                        self.fields.push(field(
                            "api_key",
                            &format!("{} API key", provider_name(kind)),
                            secret.api_key.clone().unwrap_or_default(),
                            true,
                        ));
                        self.fields.push(field(
                            "__save",
                            &format!("Save {} key", provider_name(kind)),
                            "Enter".into(),
                            false,
                        ));
                        self.fields.push(field(
                            "__test",
                            "Verify connection",
                            "Enter".into(),
                            false,
                        ));
                        self.fields.push(field(
                            "__models",
                            "Choose Writer / Tools",
                            "Models".into(),
                            false,
                        ));
                        self.fields.push(field(
                            "__advanced",
                            "Advanced endpoint",
                            if self.provider_advanced {
                                "Hide"
                            } else {
                                "Show"
                            }
                            .into(),
                            false,
                        ));
                        if self.provider_advanced {
                            self.fields.push(field(
                                "base_url",
                                "API endpoint",
                                secret.base_url,
                                false,
                            ));
                        }
                    }
                }
            }
            ModuleId::Gmail => {
                self.fields.clear();
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
        if key == "__writer_provider" || key == "__tool_provider" {
            self.open_role_provider_picker(key == "__writer_provider");
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

    pub fn role_provider_choices(&self, writer: bool) -> Vec<&'static str> {
        let mut choices = vec!["grok", "openrouter"];
        if writer {
            choices.insert(1, "openai-chatgpt");
        }
        if self.auth.account("local").is_some() {
            choices.push("local");
        }
        choices
    }

    fn open_role_provider_picker(&mut self, writer: bool) {
        let current = provider::effective_kind(&self.role_secret(writer));
        self.provider_choice = self
            .role_provider_choices(writer)
            .iter()
            .position(|kind| *kind == current)
            .unwrap_or(0);
        self.provider_picker = Some(writer);
    }

    fn on_provider_picker_key(&mut self, key: KeyEvent) -> bool {
        let Some(writer) = self.provider_picker else {
            return false;
        };
        let choices = self.role_provider_choices(writer);
        match key.code {
            KeyCode::Esc => self.provider_picker = None,
            KeyCode::Up | KeyCode::Char('k') => {
                self.provider_choice = (self.provider_choice + choices.len() - 1) % choices.len()
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.provider_choice = (self.provider_choice + 1) % choices.len()
            }
            KeyCode::Enter => {
                self.select_role_provider(writer, choices[self.provider_choice]);
                self.provider_picker = None;
            }
            _ => {}
        }
        false
    }

    fn select_role_provider(&mut self, writer: bool, kind: &str) {
        if !self.role_provider_choices(writer).contains(&kind) {
            return;
        }
        if provider::effective_kind(&self.role_secret(writer)) == kind {
            return;
        }
        let secret = provider::account_secret(&self.auth, kind);
        let mut next = self.settings.clone();
        let model = if kind == "openai-chatgpt" {
            "codex-default".into()
        } else {
            secret.model
        };
        if writer {
            next.writer_provider = kind.into();
            next.writer_model = model;
        } else {
            next.tool_provider = kind.into();
            next.tool_model = model;
        }
        match self.save_settings_config(&next) {
            Ok(()) => {
                self.settings = next;
                self.status = "Model role saved".into();
            }
            Err(err) => {
                self.status = "Model role save failed".into();
                self.log_event("system", &format!("model role save failed: {err}"));
            }
        }
        self.load_fields(ModuleId::Providers);
    }

    fn save_settings_config(&self, settings: &SettingsFile) -> Result<()> {
        #[cfg(test)]
        {
            settings.save_to(&self.test_config_home.path().join("config.toml"))
        }
        #[cfg(not(test))]
        {
            settings.save()
        }
    }

    fn save_auth_config(&self, auth: &AuthFile) -> Result<()> {
        #[cfg(test)]
        {
            auth.save_to(&self.test_config_home.path().join("auth.json"))
        }
        #[cfg(not(test))]
        {
            auth.save()
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
            self.sync_brain_list_ui();
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
        if self.module == Some(ModuleId::Providers) && self.provider_page == ProviderPage::Research
        {
            self.research_field_action(key);
            return;
        }
        if self.form_module() == Some(ModuleId::Brain) && key == "__save" {
            self.save_brain_memory();
            return;
        }
        match (self.module, key) {
            (Some(ModuleId::Providers), "__grok_subscription_login") => {
                self.start_grok_subscription(true)
            }
            (Some(ModuleId::Providers), "__grok_subscription_check") => {
                self.start_grok_subscription(false)
            }
            (Some(ModuleId::Providers), "__save") => self.save_provider_fields(),
            (Some(ModuleId::Providers), "__test") => self.test_provider(),
            (Some(ModuleId::Providers), "__models") => {
                self.select_provider_page(ProviderPage::Models)
            }
            (Some(ModuleId::Providers), "__advanced") => {
                // Keep the in-progress key while expanding the advanced field.
                let key = self.field_value("api_key");
                let endpoint = self.field_value("base_url");
                self.provider_advanced = !self.provider_advanced;
                self.load_fields(ModuleId::Providers);
                self.field_set("api_key", key);
                if !endpoint.is_empty() {
                    self.field_set("base_url", endpoint);
                }
            }
            (Some(ModuleId::Providers), "__subscription_login") => {
                if self.subscription_pending {
                    return;
                }
                self.subscription_pending = true;
                self.subscription_status = "Starting ChatGPT sign-in…".into();
                self.subscription_instructions.clear();
                let tx = self.tx.clone();
                tokio::spawn(async move {
                    let progress_tx = tx.clone();
                    let result = argos_osint_core::subscription::login(move |line| {
                        let _ = progress_tx.send(AppMsg::SubscriptionProgress(line.into()));
                    })
                    .await
                    .map_err(|err| err.to_string());
                    let _ = tx.send(AppMsg::SubscriptionCheck(result));
                });
            }
            (Some(ModuleId::Providers), "__subscription_check") => {
                if self.subscription_pending {
                    return;
                }
                self.subscription_pending = true;
                self.subscription_status = "Checking ChatGPT sign-in…".into();
                let tx = self.tx.clone();
                tokio::spawn(async move {
                    let result = argos_osint_core::subscription::check_login()
                        .await
                        .map_err(|err| err.to_string());
                    let _ = tx.send(AppMsg::SubscriptionCheck(result));
                });
            }
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
        if matches!(key.code, KeyCode::Char(_) | KeyCode::Backspace) {
            if let Some(kind) = self
                .provider_page
                .account()
                .filter(|_| self.module == Some(ModuleId::Providers))
            {
                self.provider_draft_checks.remove(kind);
                self.catalog_generation
                    .entry(kind.into())
                    .and_modify(|g| *g += 1)
                    .or_insert(1);
            }
        }
        match key.code {
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if let Some(field) = self.fields.get_mut(self.field_sel) {
                    field.value.clear();
                }
            }
            KeyCode::Esc | KeyCode::Enter => self.editing = false,
            KeyCode::Backspace => {
                if let Some(field) = self.fields.get_mut(self.field_sel) {
                    field.value.pop();
                }
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                if let Some(field) = self.fields.get_mut(self.field_sel) {
                    field.value.push(c);
                }
            }
            _ => {}
        }
        false
    }

    fn start_grok_subscription(&mut self, login: bool) {
        if self.grok_subscription_pending {
            return;
        }
        self.grok_subscription_pending = true;
        self.grok_subscription_instructions.clear();
        self.grok_subscription_status = if login {
            "Starting Grok sign-in…"
        } else {
            "Checking Grok subscription…"
        }
        .into();
        let generation = self.catalog_generation.get("grok").copied().unwrap_or(0) + 1;
        self.catalog_generation.insert("grok".into(), generation);
        let secret = provider::account_secret(&self.auth, "grok");
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = async {
                if login {
                    let progress_tx = tx.clone();
                    argos_osint_core::grok_oauth::login(move |line| {
                        let _ = progress_tx.send(AppMsg::GrokSubscriptionProgress {
                            generation,
                            line: line.into(),
                        });
                    })
                    .await
                    .map_err(|err| err.to_string())?;
                } else {
                    argos_osint_core::grok_oauth::check_login()
                        .await
                        .map_err(|err| err.to_string())?;
                }
                provider::verified_catalog(&secret)
                    .await
                    .map_err(|err| err.to_string())
            }
            .await;
            let _ = tx.send(AppMsg::GrokSubscriptionCheck { generation, result });
        });
    }

    fn provider_form_secret(&self) -> Option<ProviderSecret> {
        let kind = self.provider_page.account()?;
        if kind != "openrouter" {
            return None;
        }
        let mut secret = provider::account_secret(&self.auth, kind);
        secret.api_key =
            Some(self.field_value("api_key").trim().to_string()).filter(|key| !key.is_empty());
        if self.provider_advanced {
            secret.base_url = provider::normalize_base(&self.field_value("base_url"));
        }
        Some(secret)
    }

    fn validate_provider_endpoint(secret: &ProviderSecret) -> Result<(), String> {
        let url = url::Url::parse(&secret.base_url)
            .map_err(|_| "Enter a valid HTTPS API endpoint".to_string())?;
        if url.scheme() != "https"
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err("Use an HTTPS endpoint without credentials, query, or fragment".into());
        }
        Ok(())
    }

    fn save_provider_fields(&mut self) {
        let Some(secret) = self.provider_form_secret() else {
            return;
        };
        if let Err(err) = Self::validate_provider_endpoint(&secret) {
            self.status = err;
            return;
        }
        let kind = provider::effective_kind(&secret);
        let mut auth = self.auth.clone();
        auth.set_account(secret);
        match self.save_auth_config(&auth) {
            Ok(()) => {
                self.auth = auth;
                self.catalog_generation
                    .entry(kind.clone())
                    .and_modify(|g| *g += 1)
                    .or_insert(1);
                self.model_catalogs.remove(&kind);
                self.provider_checks.remove(&kind);
                self.provider_draft_checks.remove(&kind);
                if kind == provider::effective_kind(&self.text_secret()) {
                    self.catalog.clear();
                    self.remote_models.clear();
                }
                self.status = format!("{} account saved", provider_name(&kind));
                self.log_event("system", &self.status.clone());
            }
            Err(err) => {
                self.status = "Account save failed".into();
                self.log_event("system", &format!("provider save failed: {err}"));
            }
        }
    }

    fn test_provider(&mut self) {
        // Test the current form without implicitly saving it or changing a role.
        let Some(secret) = self.provider_form_secret() else {
            return;
        };
        if let Err(err) = Self::validate_provider_endpoint(&secret) {
            self.status = err;
            return;
        }
        let saved = provider::account_secret(&self.auth, &provider::effective_kind(&secret));
        let draft = secret.api_key != saved.api_key || secret.base_url != saved.base_url;
        self.request_catalog(secret, draft);
    }

    fn save_settings_fields(&mut self) {
        self.settings.searx_url = self.field_value("searx_url");
        self.settings.report_dir = self.field_value("report_dir");
        match self.save_settings_config(&self.settings) {
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
        let text = self.auth.redact(text);
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

pub fn provider_name(kind: &str) -> &str {
    match kind {
        "grok" => "Grok · subscription",
        "openai" => "OpenAI API",
        "openai-chatgpt" => "OpenAI · ChatGPT",
        "openrouter" => "OpenRouter",
        "local" => "Local",
        _ => kind,
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
            "Distill this completed report-network answer as one or two concise fact sentences.\n\
             Keep only an insight supported by the answer. The question sets scope; it is not evidence. Do not store or repeat the raw question or its unsupported claims.\n\
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
    app.attach_research_progress();
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
            ev = reader.next() => {
                match ev {Some(Ok(ev))=>{app.on_event(ev);},Some(Err(_))|None=>break,}
            }
            msg=inbox.recv()=>{if let Some(msg)=msg {app.on_msg(msg);for _ in 0..31 {match inbox.try_recv(){Ok(msg)=>app.on_msg(msg),Err(_)=>break}}}}
            _ = tokio::time::sleep(Duration::from_millis(200)) => app.tick(),
        }
        if app.quit {
            break;
        }
    }
    if let Some(queue) = &app.research_queue {
        queue.cancel.store(true, Ordering::Relaxed);
    }
    app.persist_pending_insights();
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

    fn attach_report(app: &mut App) {
        app.reports.push(ReportMeta {
            id: "r1".into(),
            case_id: None,
            title: "Investigation".into(),
            path: "/synthetic/report.md".into(),
            created_at: "now".into(),
        });
        app.chat_report = Some("r1".into());
    }

    fn workspace_fixture() -> (App, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("workspace.sqlite")).unwrap();
        store.ensure_session("desk", "Desk", "desk").unwrap();
        store
            .append_message("desk", "assistant", "Desk transcript remains")
            .unwrap();
        let meta = report::write_report(dir.path(), "Lovelace evidence", None,
            "# Lovelace evidence\n\n## Requirement\nwho is Ada Lovelace?\n\n## Evidence\nAda Lovelace works with Acme Corporation at example.com.\nContact ada@example.com or @ada_research at 8.8.8.8.\n\n## Themes\nTheme: computational history\n\n## Analyst note\nReview report.md and Case Desk.\n\n## Sources\nhttps://source.example/article\n").unwrap();
        store.add_report(&meta).unwrap();
        let session = format!("report:{}", meta.id);
        store
            .ensure_session(&session, &meta.title, "report")
            .unwrap();
        store
            .append_message(&session, "user", "OLD REPORT CHAT MUST NOT APPEAR")
            .unwrap();
        let mut app = App::from_parts(
            store,
            SettingsFile::default(),
            AuthFile {
                text: Some(ProviderSecret {
                    kind: "test".into(),
                    base_url: "".into(),
                    model: "test-model".into(),
                    api_key: None,
                    stt_model: None,
                    device: None,
                }),
                ..Default::default()
            },
        )
        .unwrap();
        app.open_report_chat(&meta.id);
        (app, dir)
    }

    fn workspace_key_event(app: &mut App, code: KeyCode) {
        app.on_event(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)));
    }

    fn render_workspace(app: &mut App, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| super::super::ui::draw(frame, app))
            .unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .chunks(width as usize)
            .map(|row| row.iter().map(|c| c.symbol()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn report_opens_cockpit_without_loading_or_creating_transcript() {
        let (mut app, _dir) = workspace_fixture();
        assert_eq!(app.tna_layout, TnaLayout::Cockpit);
        assert_eq!(app.focus, Focus::Graph);
        assert!(app.tna_snapshot().is_some());
        assert!(app
            .tna_snapshot()
            .unwrap()
            .nodes
            .iter()
            .any(|n| n.kind == TnaNodeKind::Topic && n.label == "computational history"));
        assert!(app.transcript().is_empty());
        assert!(!app.transcripts.contains_key(&app.session_id()));
        assert!(app.case_pages().is_empty());
        let text = render_workspace(&mut app, 120, 40);
        assert!(text.contains("NETWORK · Lovelace evidence"), "{text}");
        assert!(text.contains("Cockpit") && text.contains("ask the open graph…"));
        assert!(
            !text.contains("OLD REPORT CHAT")
                && !text.contains("Desk transcript remains")
                && !text.contains("Report chat")
        );
        assert!(app.case_tab_hits.is_empty());
        app.close_report_workspace();
        assert!(app
            .transcript()
            .iter()
            .any(|l| l.body == "Desk transcript remains"));
        assert_eq!(app.case_pages(), [CasePage::Closed, CasePage::Brain]);
        app.open_module(ModuleId::Brain);
        assert_eq!(app.case_page, CasePage::Brain);
        app.select_case_page(CasePage::Closed);
        assert!(app.routes_desk_message());
    }

    #[test]
    fn report_source_loading_is_independent_of_questions_and_historic_reading() {
        let (mut app, _dir) = workspace_fixture();
        let id = app.chat_report.clone().unwrap();
        let generation = app.report_source_generation;
        app.turn_generation = app.turn_generation.wrapping_add(1);
        app.on_msg(AppMsg::ReportSource {
            report_id: id.clone(),
            generation,
            version: None,
            result: Ok("Latest graph source".into()),
        });
        app.on_msg(AppMsg::ReportSource {
            report_id: id,
            generation,
            version: Some(1),
            result: Ok("Historic cited source".into()),
        });
        assert_eq!(app.tna_source, "Latest graph source");
        assert_eq!(
            app.report_read_source.as_deref(),
            Some("Historic cited source")
        );
        app.workspace_reading = true;
        let text = render_workspace(&mut app, 54, 18);
        assert!(text.contains("Historic cited source"));
        assert!(!text.contains("Latest graph source"));
    }

    #[test]
    fn stale_graph_results_cannot_expand_the_current_scope() {
        let (mut app, _dir) = workspace_fixture();
        let count = app.tna_snapshot().unwrap().nodes.len();
        app.on_msg(AppMsg::TnaReady {
            targeted: true,
            report_id: app.chat_report.clone(),
            scope: Some(argos_osint_core::evidence::EvidenceScope::Collection),
            result: Ok(TnaSnapshot::empty(
                argos_osint_core::tna::TnaScope::Collection,
            )),
        });
        assert_eq!(app.tna_snapshot().unwrap().nodes.len(), count);
    }

    #[test]
    fn five_layout_shortcuts_find_typing_and_navigation() {
        let (mut app, _dir) = workspace_fixture();
        for (key, layout) in [
            ('q', TnaLayout::Clusters),
            ('p', TnaLayout::Path),
            ('m', TnaLayout::Matrix),
            ('r', TnaLayout::Ribbon),
            ('g', TnaLayout::Cockpit),
        ] {
            workspace_key_event(&mut app, KeyCode::Char(key));
            assert_eq!(app.tna_layout, layout);
        }
        workspace_key_event(&mut app, KeyCode::Left);
        assert_eq!(app.tna_layout, TnaLayout::Ribbon);
        workspace_key_event(&mut app, KeyCode::Right);
        assert_eq!(app.tna_layout, TnaLayout::Cockpit);
        workspace_key_event(&mut app, KeyCode::Char('/'));
        workspace_key_event(&mut app, KeyCode::Char('q'));
        assert_eq!(app.tna_find.as_deref(), Some("q"));
        assert_eq!(app.tna_layout, TnaLayout::Cockpit);
        workspace_key_event(&mut app, KeyCode::Enter);
        assert!(!app.tna_find_editing);
        workspace_key_event(&mut app, KeyCode::Esc);
        assert!(app.tna_find.is_none());
        app.focus = Focus::Prompt;
        app.prompt = "who is ".into();
        app.cursor = app.prompt.chars().count();
        workspace_key_event(&mut app, KeyCode::Char('p'));
        assert_eq!(app.prompt, "who is p");
        assert_eq!(app.tna_layout, TnaLayout::Cockpit);
        let cursor = app.cursor;
        workspace_key_event(&mut app, KeyCode::Left);
        assert_eq!(app.cursor, cursor - 1);
        assert_eq!(app.tna_layout, TnaLayout::Cockpit);
    }

    #[tokio::test]
    async fn discussion_is_ephemeral_and_explicitly_retained_insight_survives_restart() {
        let (mut app, dir) = workspace_fixture();
        let report_id = app.chat_report.clone().unwrap();
        let session = app.session_id();
        let before = app.store.load_messages(&session).unwrap();
        app.prompt = "who is Ada Lovelace".into();
        app.cursor = app.prompt.len();
        app.submit();
        assert!(
            app.confirm_query.is_none() && app.scope.is_none() && app.pending_reports.is_empty()
        );
        assert!(app.history.is_empty());
        assert!(app.transcript().is_empty());
        assert_eq!(
            app.tna_answer.as_ref().unwrap().question,
            "who is Ada Lovelace"
        );
        assert!(!app.running); // ordinary evidence retrieval makes no model/provider call
        assert!(app
            .tna_answer
            .as_ref()
            .unwrap()
            .answer
            .contains("Supported saved material"));
        app.tna_answer = Some(TnaAnswer {
            question: "who is Ada Lovelace".into(),
            pending: true,
            ..Default::default()
        });
        app.cancel.store(false, Ordering::Relaxed);
        app.running = true; // simulate a separately requested model discussion
        let generation = app.turn_generation;
        app.on_msg(AppMsg::ScopedTurn {
            generation,
            report_id: Some(report_id.clone()),
            event: TurnEvent::Delta("Ada Lovelace works with ".into()),
        });
        assert_eq!(
            app.tna_answer.as_ref().unwrap().answer,
            "Ada Lovelace works with "
        );
        assert!(app.memories.is_empty());
        let text = render_workspace(&mut app, 120, 40);
        assert!(
            text.contains("TNA · Lovelace evidence") && text.contains("Ada Lovelace works with"),
            "{text}"
        );
        assert!(text.contains("ask the open graph…"));
        app.on_msg(AppMsg::ScopedTurn {
            generation,
            report_id: Some(report_id.clone()),
            event: TurnEvent::Done(
                "Ada Lovelace works with Acme Corporation in this report.".into(),
            ),
        });
        assert!(!app.tna_answer.as_ref().unwrap().filed);
        assert!(app.store.list_memories().unwrap().is_empty());
        app.run_slash("/retain-answer");
        assert!(app.tna_answer.as_ref().unwrap().filed);
        let facts = app.store.list_memories().unwrap();
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].report_id.as_deref(), Some(report_id.as_str()));
        assert!(!facts[0].text.contains("who is Ada"));
        assert_eq!(
            app.store.load_messages(&session).unwrap().len(),
            before.len()
        );
        app.persist_visible_chat();
        assert_eq!(
            app.store.load_messages(&session).unwrap().len(),
            before.len()
        );
        app.set_tna_layout(TnaLayout::Ribbon);
        assert!(app.tna_answer.is_some());
        app.run_slash("/clear");
        assert!(app.tna_answer.is_none());
        assert_eq!(app.store.list_memories().unwrap().len(), 1);
        app.close_report_workspace();
        drop(app);
        let mut restarted = App::from_parts(
            Store::open(&dir.path().join("workspace.sqlite")).unwrap(),
            SettingsFile::default(),
            AuthFile::default(),
        )
        .unwrap();
        assert!(restarted.tna_answer.is_none() && restarted.chat_report.is_none());
        restarted.open_module(ModuleId::Brain);
        assert_eq!(restarted.memories.len(), 1);
        assert_eq!(
            restarted.memories[0].report_id.as_deref(),
            Some(report_id.as_str())
        );
    }

    #[test]
    fn cancelled_failed_and_stale_tna_turns_never_file_insights() {
        let (mut app, _dir) = workspace_fixture();
        app.tna_answer = Some(TnaAnswer {
            question: "Who?".into(),
            pending: true,
            ..Default::default()
        });
        app.running = true;
        let generation = app.turn_generation;
        let report_id = app.chat_report.clone();
        app.on_turn(TurnEvent::Delta("partial".into()));
        app.cancel_or_quit();
        app.on_msg(AppMsg::ScopedTurn {
            generation,
            report_id: report_id.clone(),
            event: TurnEvent::Done("Late completion".into()),
        });
        assert!(app.memories.is_empty());
        assert!(app.tna_answer.as_ref().unwrap().error.is_some());
        app.tna_answer = Some(TnaAnswer {
            question: "Who?".into(),
            pending: true,
            ..Default::default()
        });
        app.cancel = Arc::new(AtomicBool::new(false));
        app.on_turn(TurnEvent::Failed("provider failure".into()));
        app.on_turn(TurnEvent::Done("Should be ignored".into()));
        assert!(app.memories.is_empty());
        app.on_turn(TurnEvent::Memory(Memory::fact(
            "raw",
            "raw user question",
            "",
        )));
        assert!(app.memories.is_empty());
        app.close_report_workspace();
        app.on_msg(AppMsg::ScopedTurn {
            generation,
            report_id,
            event: TurnEvent::Delta("LEAK".into()),
        });
        assert!(!app.transcript().iter().any(|l| l.body.contains("LEAK")));
    }

    #[test]
    fn escape_order_and_desk_only_actions() {
        let (mut app, _dir) = workspace_fixture();
        app.tna_find = Some("Ada".into());
        app.tna_find_editing = true;
        app.tna_answer = Some(TnaAnswer {
            question: "Who?".into(),
            answer: "Answer".into(),
            ..Default::default()
        });
        workspace_key_event(&mut app, KeyCode::Esc);
        assert!(app.tna_find.is_none());
        assert!(app.tna_answer.is_some() && app.chat_report.is_some());
        workspace_key_event(&mut app, KeyCode::Esc);
        assert!(app.tna_answer.is_none() && app.chat_report.is_some());
        for cmd in [
            "/brain",
            "/brain fact raw question",
            "/new who is Ada",
            "/search Ada",
            "/report copy",
            "/use missing",
        ] {
            app.run_slash(cmd);
            assert!(app.chat_report.is_some());
            assert!(app.scope.is_none());
            assert_eq!(app.case_page, CasePage::Network);
        }
        app.open_module(ModuleId::Brain);
        assert_eq!(app.case_page, CasePage::Network);
        assert!(!app.brain_card);
        workspace_key_event(&mut app, KeyCode::Char('+'));
        assert!(app.scope.is_none());
        workspace_key_event(&mut app, KeyCode::Esc);
        assert!(app.chat_report.is_none());
        assert_eq!(app.focus, Focus::Prompt);
        app.prompt = "who is Ada Lovelace".into();
        workspace_key_event(&mut app, KeyCode::Char('+'));
        assert!(app.scope.is_some());
    }

    #[test]
    fn path_pins_rank_simple_paths_and_limit_hops() {
        let mut app = five_hop_report_app();
        app.set_tna_layout(TnaLayout::Path);
        workspace_key_event(&mut app, KeyCode::Char('f'));
        assert_eq!(app.tna_from.as_deref(), Some("a"));
        assert!(app.tna_paths().is_empty());
        app.tna_focus_entity("e");
        workspace_key_event(&mut app, KeyCode::Char('t'));
        let paths = app.tna_paths();
        assert_eq!(paths[0].nodes, ["a", "b", "c", "d", "e"]);
        app.tna_to = Some("f".into());
        assert!(app.tna_paths().is_empty());
        let snap = app.tna_report.as_mut().unwrap();
        snap.edges.extend(
            [("a", "c", 4), ("c", "e", 4), ("a", "d", 1), ("d", "e", 1)].map(|(a, b, weight)| {
                argos_osint_core::tna::TnaEdge {
                    from: a.into(),
                    to: b.into(),
                    weight,
                }
            }),
        );
        app.tna_to = Some("e".into());
        let paths = app.tna_paths();
        assert!(paths.len() <= 5);
        assert_eq!(paths[0].nodes, ["a", "c", "e"]);
        for pair in paths.windows(2) {
            assert!(pair[0].nodes.len() <= pair[1].nodes.len());
            if pair[0].nodes.len() == pair[1].nodes.len() {
                assert!(pair[0].strength >= pair[1].strength);
            }
        }
        for path in paths {
            let unique: std::collections::HashSet<_> = path.nodes.iter().collect();
            assert_eq!(unique.len(), path.nodes.len());
            assert!(path.nodes.len() <= 5);
        }
        app.tna_to = Some("h".into());
        assert!(app.tna_paths().is_empty());
    }

    #[test]
    fn matrix_filter_navigation_and_ribbon_provenance() {
        let (mut app, _dir) = workspace_fixture();
        app.set_tna_layout(TnaLayout::Matrix);
        let nodes = app.tna_matrix_nodes();
        assert!(nodes.len() <= 24);
        workspace_key_event(&mut app, KeyCode::Char('l'));
        assert_eq!(app.tna_matrix_col, 1);
        workspace_key_event(&mut app, KeyCode::Char('j'));
        assert_eq!(app.tna_matrix_row, 1);
        let selected = app.tna_snapshot().unwrap().nodes[nodes[1]].id.clone();
        workspace_key_event(&mut app, KeyCode::Enter);
        assert_eq!(app.tna_layout, TnaLayout::Cockpit);
        assert_eq!(app.tna_focus_id.as_deref(), Some(selected.as_str()));
        app.tna_find = Some("Ada Lovelace".into());
        let filtered = app.tna_matrix_nodes();
        assert_eq!(filtered.len(), 1);
        app.tna_find = None;
        app.set_tna_layout(TnaLayout::Ribbon);
        let accepted = app.tna_ribbon_decisions().len();
        workspace_key_event(&mut app, KeyCode::Char('d'));
        assert!(app.tna_show_rejected);
        assert!(app.tna_ribbon_decisions().len() >= accepted);
        let decision = app
            .tna_ribbon_decisions()
            .iter()
            .find(|d| d.canonical_id.is_some())
            .unwrap()
            .to_owned();
        assert_eq!(
            app.tna_source.get(decision.start..decision.end),
            Some(decision.original.as_str())
        );
        let nearby = app.tna_ribbon_window(decision.start);
        assert!(nearby.len() <= 3);
        assert!(nearby.iter().all(|d| d.canonical_id.is_some()));
        let snap = app.tna_snapshot().unwrap();
        let edge = snap.edges.first().unwrap();
        let evidence = app.tna_edge_evidence(&edge.from, &edge.to);
        assert!(
            evidence.contains("bytes") && !evidence.contains("Evidence unavailable"),
            "{evidence}"
        );
        workspace_key_event(&mut app, KeyCode::Char('l'));
        assert_eq!(app.tna_ribbon_pos, 1);
    }

    #[test]
    fn all_workspace_layouts_render_at_normal_and_narrow_sizes() {
        let (mut app, dir) = workspace_fixture();
        for (width, height) in [(160, 48), (120, 40), (80, 24), (60, 18)] {
            for layout in TnaLayout::ALL {
                app.set_tna_layout(layout);
                if layout == TnaLayout::Path {
                    app.tna_from = app
                        .tna_snapshot()
                        .unwrap()
                        .nodes
                        .first()
                        .map(|n| n.id.clone());
                    app.tna_to = app
                        .tna_snapshot()
                        .unwrap()
                        .nodes
                        .last()
                        .map(|n| n.id.clone());
                }
                if layout == TnaLayout::Ribbon {
                    app.tna_show_rejected = true;
                }
                let text = render_workspace(&mut app, width, height);
                assert!(
                    text.contains("NETWORK") && text.contains("ask the open graph…"),
                    "{width}x{height} {}\n{text}",
                    layout.title()
                );
                assert!(!text.contains("OLD REPORT CHAT"));
                if width == 120 && height == 40 {
                    std::fs::write(dir.path().join(format!("{}.txt", layout.title())), &text)
                        .unwrap();
                    println!("120x40 {}\n{text}", layout.title());
                }
            }
        }
        app.set_tna_layout(TnaLayout::Cockpit);
        let text = render_workspace(&mut app, 160, 48);
        assert!(
            text.contains("Entity list")
                && text.contains("Ego network")
                && text.contains("Link Ledger")
        );
        let text = render_workspace(&mut app, 80, 24);
        assert!(text.contains("Entity list") && !text.contains("Link Ledger"));
        let text = render_workspace(&mut app, 80, 16);
        assert!(text.contains("Link Ledger"));
    }
    #[test]
    fn completed_insight_survives_shutdown_during_distillation_and_is_deduplicated() {
        let (mut app, dir) = workspace_fixture();
        let id = app.chat_report.clone().unwrap();
        app.pending_tna_insights.insert(
            app.turn_generation,
            (id.clone(), "Completed evidence answer".into()),
        );
        app.close_report_workspace();
        app.persist_pending_insights();
        assert!(app.pending_tna_insights.is_empty());
        app.store_report_insight(id.clone(), "Completed evidence answer".into(), 0);
        assert_eq!(app.store.list_memories().unwrap().len(), 1);
        drop(app);
        let restarted = App::from_parts(
            Store::open(&dir.path().join("workspace.sqlite")).unwrap(),
            SettingsFile::default(),
            AuthFile::default(),
        )
        .unwrap();
        assert_eq!(restarted.memories.len(), 1);
        assert_eq!(
            restarted.memories[0].report_id.as_deref(),
            Some(id.as_str())
        );
        assert!(restarted.tna_answer.is_none());
    }

    #[test]
    fn missing_source_is_visible_and_rebuilds_do_not_block_cached_report_opening() {
        let (mut app, _dir) = workspace_fixture();
        let id = app.chat_report.clone().unwrap();
        let expected = app.store.get_tna_graph(&report_key(&id)).unwrap().unwrap();
        app.close_report_workspace();
        app.tna_rebuilding = true;
        app.tna_pending_report = Some("another-report".into());
        app.open_report_chat(&id);
        assert_eq!(app.tna_snapshot().unwrap(), &expected);
        let path = app.open_report().unwrap().path.clone();
        std::fs::remove_file(path).unwrap();
        app.close_report_workspace();
        app.open_report_chat(&id);
        assert!(app.tna_source_error.is_some());
        app.spawn_turn("who is Ada Lovelace".into());
        assert!(app
            .tna_answer
            .as_ref()
            .unwrap()
            .error
            .as_ref()
            .unwrap()
            .contains("evidence unavailable"));
        assert!(!app.running && app.memories.is_empty());
    }

    #[test]
    fn workspace_context_is_small_and_source_windows_are_exact() {
        let (mut app, _dir) = workspace_fixture();
        let person = app
            .tna_snapshot()
            .unwrap()
            .nodes
            .iter()
            .find(|n| n.kind == TnaNodeKind::Person)
            .unwrap()
            .id
            .clone();
        app.tna_focus_entity(&person);
        app.tna_from = Some(person.clone());
        app.tna_to = app
            .tna_snapshot()
            .unwrap()
            .nodes
            .iter()
            .find(|n| n.kind == TnaNodeKind::Domain)
            .map(|n| n.id.clone());
        app.set_tna_layout(TnaLayout::Path);
        let context = app.view_context();
        assert!(
            context.contains("Evidence-only")
                && context.contains("Layout: Path")
                && context.contains("Path FROM: Ada Lovelace")
                && context.contains("TO: example.com")
        );
        assert!(context.contains("degree") && context.contains("Neighbors"));
        assert!(!context.contains("OLD REPORT CHAT"));
        assert!(context.len() < 2200);
        let evidence = app.tna_report_material(app.open_report().unwrap());
        assert!(evidence.contains("Focused source:") && !evidence.contains("OLD REPORT CHAT"));
        // A span outside the original 3-mention window must never be quoted as joint evidence.
        let snap = app.tna_report.as_mut().unwrap();
        snap.decisions = (0..5)
            .map(|i| argos_osint_core::tna::TnaDecision {
                report_id: "r1".into(),
                section: "Evidence".into(),
                start: i * 2,
                end: i * 2 + 1,
                original: char::from(b'a' + i as u8).to_string(),
                kind: TnaNodeKind::Handle,
                label: Some(format!("n{i}")),
                canonical_id: Some(format!("n{i}")),
                reason: "accepted".into(),
            })
            .collect();
        app.chat_report = Some("r1".into());
        snap.scope = argos_osint_core::tna::TnaScope::Targeted {
            report_id: "r1".into(),
            title: "test".into(),
        };
        app.tna_source = "a b c d e".into();
        assert!(app
            .tna_edge_evidence("n0", "n4")
            .contains("No joint excerpt recovered"));
        assert!(app.tna_edge_evidence("n0", "n2").contains("a b c"));
        app.set_tna_layout(TnaLayout::Ribbon);
        app.tna_source =
            "## Evidence\nfirst line without entities\nlast line without entities\n".into();
        workspace_key_event(&mut app, KeyCode::Char('l'));
        workspace_key_event(&mut app, KeyCode::Char('l'));
        assert_eq!(app.tna_ribbon_pos, 2);
    }
    #[test]
    fn cockpit_neighbor_cycle_keeps_the_selected_link_visible() {
        let mut app = five_hop_report_app();
        app.tna_focus_entity("b");
        workspace_key_event(&mut app, KeyCode::Char(']'));
        assert_eq!(app.tna_ledger_neighbor(), Some("c"));
        let boxes = app.tna_detail_box_items(2);
        assert!(boxes
            .iter()
            .any(|TnaDisplayItem::Real { idx }| app.tna_snapshot().unwrap().nodes[*idx].id == "c"));
        assert_eq!(app.tna_focus_id.as_deref(), Some("b"));
    }

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
        assert!(text.contains("saves a case without a report"), "{text}");
        assert!(text.contains("Ask saved"), "{text}");
        assert!(
            !text.contains("Reports  Brain") && !text.contains("Reports Brain"),
            "{text}"
        );
        assert!(text.contains("Ctrl+P"), "{text}");
        assert_eq!(super::super::slash_menu("/use").option_count(), 1);
        let chat = app.canvas_area;
        app.click(chat.x + 2, chat.y + 2);
        assert_eq!(app.focus, Focus::Canvas);
        assert_eq!(app.case_tab_hits.len(), 2);
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
    fn brain_list_scroll_state_tracks_selection() {
        let store = Store::memory().unwrap();
        let mut app = App::from_parts(store, SettingsFile::default(), AuthFile::default()).unwrap();
        for i in 0..12 {
            app.memories.push(Memory::fact(
                format!("m{i}"),
                format!("memory-row-{i}"),
                "t",
            ));
        }
        app.open_module(ModuleId::Brain);
        assert_eq!(app.case_page, CasePage::Brain);
        app.sync_brain_list_ui();
        assert_eq!(app.brain_list_state.selected(), Some(0));
        app.move_brain_sel(1);
        assert_eq!(app.brain_sel, 1);
        assert_eq!(app.brain_list_state.selected(), Some(1));
        // Mid-list: selection and scrollbar stay aligned (ITEM_HEIGHT = 1).
        for _ in 0..5 {
            app.move_brain_sel(1);
        }
        assert_eq!(app.brain_sel, 6);
        assert_eq!(app.brain_list_state.selected(), Some(6));
        // Clamp when list shrinks.
        app.memories.truncate(3);
        app.sync_brain_list_ui();
        assert_eq!(app.brain_sel, 2);
        assert_eq!(app.brain_list_state.selected(), Some(2));
        app.memories.clear();
        app.sync_brain_list_ui();
        assert_eq!(app.brain_sel, 0);
        assert_eq!(app.brain_list_state.selected(), None);
        // Render long list: scrollbar + selection symbol visible.
        for i in 0..20 {
            app.memories.push(Memory::fact(
                format!("r{i}"),
                format!("scroll-fact-{i}"),
                "t",
            ));
        }
        app.brain_sel = 10;
        app.sync_brain_list_ui();
        let backend = TestBackend::new(100, 24);
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
        assert!(text.contains("Memories"), "{text}");
        assert!(text.contains("Enter views a fact"), "{text}");
        assert!(text.contains("scroll-fact-10"), "{text}");
        assert_eq!(app.brain_list_state.selected(), Some(10));
    }

    #[test]
    fn case_pages_keep_network_inside_open_report() {
        let pages = CasePage::all();
        assert_eq!(pages.len(), 2);
        assert!(!pages.contains(&CasePage::Network));
        assert_eq!(CasePage::Network.title(), "Network");
        assert_eq!(pages.map(|p| p.title()), ["Desk", "Brain"]);
    }

    #[test]
    fn slash_network_opens_network_page() {
        let store = Store::memory().unwrap();
        let mut app = App::from_parts(store, SettingsFile::default(), AuthFile::default()).unwrap();
        app.run_slash("/network");
        assert_eq!(app.case_page, CasePage::Closed);
        assert!(!app.case_pages().contains(&CasePage::Network));
        attach_report(&mut app);
        app.run_slash("/network");
        assert_eq!(app.case_page, CasePage::Network);
        assert_eq!(app.focus, Focus::Graph);
    }

    fn five_hop_report_app() -> App {
        let store = Store::memory().unwrap();
        let mut app = App::from_parts(store, SettingsFile::default(), AuthFile::default()).unwrap();
        attach_report(&mut app);
        let mut snap = TnaSnapshot::empty(argos_osint_core::tna::TnaScope::Targeted {
            report_id: "r1".into(),
            title: "Investigation".into(),
        });
        snap.nodes = [
            ("a", "Ada Lovelace", TnaNodeKind::Person),
            ("b", "example.com", TnaNodeKind::Domain),
            ("c", "ada@example.com", TnaNodeKind::Email),
            ("d", "third_hop", TnaNodeKind::Handle),
            ("e", "fourth_hop", TnaNodeKind::Handle),
            ("f", "fifth_hop", TnaNodeKind::Handle),
            ("g", "sixth_hop", TnaNodeKind::Handle),
            ("h", "isolated", TnaNodeKind::Handle),
        ]
        .into_iter()
        .map(|(id, label, kind)| TnaNode {
            id: id.into(),
            label: label.into(),
            kind,
            cluster: kind.cluster(),
            mentions: 1,
            degree: 1,
            x: 0.5,
            y: 0.5,
        })
        .collect();
        snap.edges = [
            ("a", "b"),
            ("b", "c"),
            ("c", "d"),
            ("d", "e"),
            ("e", "f"),
            ("f", "g"),
        ]
        .into_iter()
        .map(|(a, b)| argos_osint_core::tna::TnaEdge {
            from: a.into(),
            to: b.into(),
            weight: 1,
        })
        .collect();
        app.tna_report = Some(snap);
        app.run_slash("/network");
        app
    }

    #[test]
    fn tna_details_follow_five_hops_from_selected_focus() {
        let mut app = five_hop_report_app();
        let visible: Vec<_> = app
            .tna_display_nodes()
            .into_iter()
            .map(|TnaDisplayItem::Real { idx }| app.tna_snapshot().unwrap().nodes[idx].id.clone())
            .collect();
        assert_eq!(visible, ["a", "b", "c", "d", "e", "f"]);
        app.focus = Focus::TableDetail;
        for _ in 0..4 {
            app.on_tna_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
        }
        let page = app.tna_detail_box_items(3);
        assert!(page
            .iter()
            .any(|TnaDisplayItem::Real { idx }| app.tna_snapshot().unwrap().nodes[*idx].id == "f"));
        app.focus = Focus::Graph;
        app.on_tna_key(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::NONE));
        assert_eq!(app.tna_focus_id.as_deref(), Some("a"));
        app.on_tna_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
        assert_eq!(app.tna_focus_id.as_deref(), Some("b"));
        assert!(app
            .tna_display_nodes()
            .iter()
            .any(|TnaDisplayItem::Real { idx }| app.tna_snapshot().unwrap().nodes[*idx].id == "g"));
        app.focus = app.next_focus();
        assert_eq!(app.focus, Focus::TableDetail);
        app.focus = app.next_focus();
        assert_eq!(app.focus, Focus::Prompt);
    }

    #[test]
    fn tna_table_render_has_type_borders_and_values_inside_boxes() {
        let mut app = five_hop_report_app();
        for (width, height) in [(120, 40), (100, 30)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal
                .draw(|frame| super::super::ui::draw(frame, &mut app))
                .unwrap();
            let text = terminal
                .backend()
                .buffer()
                .content()
                .chunks(width as usize)
                .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
                .collect::<Vec<_>>()
                .join("\n");
            println!("{width}x{height}\n{text}");
            assert!(
                text.contains("Ada Lovelace") && text.contains("Ego network · 5 hops"),
                "{text}"
            );
            assert!(
                !text.contains("v views")
                    && !text.contains("Most connected entities")
                    && !text.contains("· outline")
                    && !text.contains("type group")
                    && !text.contains("source mentions")
                    && !text.contains("most connected:")
                    && !text.contains("unconnected types:"),
                "{text}"
            );
            let border = text
                .lines()
                .position(|row| row.contains("╭*person"))
                .expect("person type in box border");
            assert!(
                text.lines()
                    .nth(border + 1)
                    .unwrap()
                    .contains("Ada Lovelace"),
                "entity value must be inside box: {text}"
            );
            if width == 120 {
                assert!(
                    text.contains("╭email"),
                    "second-hop email type border: {text}"
                );
            }
        }
    }

    #[test]
    fn network_tab_disappears_when_report_closes_and_never_uses_other_report() {
        let mut app = five_hop_report_app();
        assert!(app.case_pages().is_empty());
        app.chat_report = Some("other".into());
        assert!(app.tna_snapshot().is_none());
        app.chat_report = Some("r1".into());
        app.on_esc();
        assert_eq!(app.case_page, CasePage::Closed);
        assert!(!app.case_pages().contains(&CasePage::Network));
        assert!(app.tna_snapshot().is_none());
        app.run_slash("/find");
        assert!(app.tna_find.is_none());
        app.cycle_group_page(1);
        assert_eq!(app.case_page, CasePage::Brain);
        app.cycle_group_page(1);
        assert_eq!(app.case_page, CasePage::Closed);
    }

    #[test]
    fn report_network_old_cache_rebuilds_on_first_use() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::memory().unwrap();
        let report = argos_osint_core::report::write_report(
            dir.path(),
            "Ada",
            None,
            "## Evidence\nAda Lovelace operates example.com.\n",
        )
        .unwrap();
        store.add_report(&report).unwrap();
        let mut old = TnaSnapshot::empty(argos_osint_core::tna::TnaScope::Targeted {
            report_id: report.id.clone(),
            title: report.title.clone(),
        });
        old.pipeline_version = 0;
        old.title = "Old noisy graph".into();
        store
            .upsert_tna_graph(&report_key(&report.id), &old)
            .unwrap();
        let mut app = App::from_parts(store, SettingsFile::default(), AuthFile::default()).unwrap();
        app.open_report_chat(&report.id);
        app.run_slash("/network");
        let current = app.tna_snapshot().unwrap().clone();
        assert_eq!(
            current.pipeline_version,
            argos_osint_core::tna::PIPELINE_VERSION
        );
        assert_ne!(current.title, old.title);
        assert!(current.nodes.iter().any(|n| n.label == "Ada Lovelace"));
        app.ensure_tna_snapshot(false);
        assert_eq!(app.tna_snapshot().unwrap(), &current);
        let persisted = app
            .store
            .get_tna_graph(&report_key(&report.id))
            .unwrap()
            .unwrap();
        assert_eq!(persisted.built_at, current.built_at);
        assert_eq!(persisted.decisions, current.decisions);
        assert_eq!(persisted.nodes.len(), current.nodes.len());
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
    fn tna_graph_box_budget_caps_display() {
        // Star: focus + 30 leaves → ego1 alone is 31; Graph boxes must stay ≤ budget.
        let mut nodes = vec![TnaNode {
            id: "F".into(),
            label: "focus".into(),
            kind: TnaNodeKind::Person,
            cluster: TnaCluster::Identity,
            mentions: 1,
            degree: 30,
            x: 0.5,
            y: 0.5,
        }];
        let mut edges = Vec::new();
        for i in 0..30 {
            let id = format!("n{i}");
            nodes.push(TnaNode {
                id: id.clone(),
                label: id.clone(),
                kind: TnaNodeKind::Handle,
                cluster: TnaCluster::Identity,
                mentions: 1,
                degree: 1,
                x: 0.1 * ((i % 10) as f64),
                y: 0.1 * ((i / 10) as f64),
            });
            edges.push(argos_osint_core::tna::TnaEdge {
                from: "F".into(),
                to: id,
                weight: 1,
            });
        }
        let snap = TnaSnapshot {
            pipeline_version: 2,
            decisions: Vec::new(),
            scope: argos_osint_core::tna::TnaScope::Collection,
            title: "TNA · budget".into(),
            nodes,
            edges,
            clusters: vec![],
            anchors: vec![],
            gaps: vec![],
            built_at: "t".into(),
        };
        let store = Store::memory().unwrap();
        let mut app = App::from_parts(store, SettingsFile::default(), AuthFile::default()).unwrap();
        attach_report(&mut app);
        let mut snap = snap;
        snap.scope = argos_osint_core::tna::TnaScope::Targeted {
            report_id: "r1".into(),
            title: "test".into(),
        };
        app.tna_report = Some(snap);
        app.run_slash("/network");
        // select_case_page clears focus/expanded — restore after open.
        app.tna_focus_id = Some("F".into());
        let items = app.tna_detail_box_items(TNA_GRAPH_BOX_BUDGET);
        assert!(
            items.len() <= TNA_GRAPH_BOX_BUDGET,
            "budget {} exceeded: {}",
            TNA_GRAPH_BOX_BUDGET,
            items.len()
        );
        assert!(!items.is_empty());
    }

    #[test]
    fn tna_table_master_detail_focus_and_scroll_independent() {
        let store = Store::memory().unwrap();
        let mut app = App::from_parts(store, SettingsFile::default(), AuthFile::default()).unwrap();
        attach_report(&mut app);
        app.tna_report = Some(TnaSnapshot {
            pipeline_version: 2,
            decisions: Vec::new(),
            scope: argos_osint_core::tna::TnaScope::Targeted {
                report_id: "r1".into(),
                title: "table".into(),
            },
            title: "TNA · table".into(),
            nodes: (0..5)
                .map(|i| TnaNode {
                    id: format!("n{i}"),
                    label: format!("node-{i}"),
                    kind: TnaNodeKind::Domain,
                    cluster: TnaCluster::Infrastructure,
                    mentions: i as u32 + 1,
                    degree: i as u32,
                    x: 0.1 * i as f64,
                    y: 0.5,
                })
                .collect(),
            edges: vec![argos_osint_core::tna::TnaEdge {
                from: "n0".into(),
                to: "n1".into(),
                weight: 1,
            }],
            clusters: vec![],
            anchors: vec![],
            gaps: vec![],
            built_at: "t".into(),
        });
        app.run_slash("/network");
        assert_eq!(app.focus, Focus::Graph);
        assert_eq!(app.tna_sel, 0);
        app.on_tna_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
        assert_eq!(app.tna_sel, 1);
        assert_eq!(app.tna_table_state.selected(), Some(1));
        assert_eq!(app.tna_focus_id.as_deref(), Some("n1"));
        // Tab cycles list → detail in Table mode.
        app.focus = app.next_focus();
        assert_eq!(app.focus, Focus::TableDetail);
        let list_sel = app.tna_sel;
        app.on_tna_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
        assert_eq!(app.tna_sel, list_sel, "detail j/k must not move list");
        assert_eq!(app.tna_detail_scroll, 1);
        app.on_tna_key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE));
        // Filter empty-state + reclamp.
        app.focus = Focus::Graph;
        app.tna_find = Some("zzz-nomatch".into());
        app.clamp_tna_sel();
        assert_eq!(app.tna_visible_nodes().len(), 0);
        assert_eq!(app.tna_table_state.selected(), None);
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
        assert!(
            text.contains("No nodes") || text.contains("Filter"),
            "{text}"
        );
        app.tna_find = None;
        app.clamp_tna_sel();
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
        assert!(
            text.contains("Entity list") || text.contains("Ego network"),
            "{text}"
        );
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
        app.catalog_generation.insert("grok".into(), 1);
        app.on_msg(AppMsg::ModelList {
            kind: "grok".into(),
            generation: 1,
            draft: false,
            result: Err(format!("provider {marker}")),
        });

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
        assert_eq!(app.provider_page, ProviderPage::Models);
        assert_eq!(labels, vec!["Provider", "Model", "Provider", "Model"]);
    }

    fn key(code: KeyCode) -> Event {
        Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn provider_fixture() -> App {
        let mut auth = AuthFile::default();
        for (kind, key) in [
            ("grok", "xai-SAVED-PRIVATE"),
            ("openrouter", "router-SAVED-PRIVATE"),
        ] {
            let mut secret = provider::account_secret(&auth, kind);
            secret.api_key = Some(key.into());
            auth.set_account(secret);
        }
        let settings = SettingsFile {
            writer_provider: "grok".into(),
            writer_model: "grok-writer".into(),
            tool_provider: "openrouter".into(),
            tool_model: "vendor/research".into(),
            ..Default::default()
        };
        App::from_parts(Store::memory().unwrap(), settings, auth).unwrap()
    }

    #[test]
    fn provider_account_forms_do_not_copy_or_overwrite_keys() {
        let mut app = provider_fixture();
        let role_settings = serde_json::to_value(&app.settings).unwrap();
        app.select_provider_page(ProviderPage::Grok);
        let before = serde_json::to_value(&app.auth).unwrap();
        assert!(!app
            .fields
            .iter()
            .any(|f| matches!(f.key.as_str(), "api_key" | "base_url" | "__save" | "__test")));
        assert!(app
            .fields
            .iter()
            .any(|f| f.key == "__grok_subscription_login"));
        assert!(app
            .fields
            .iter()
            .any(|f| f.key == "__grok_subscription_check"));
        app.save_provider_fields();
        assert_eq!(serde_json::to_value(&app.auth).unwrap(), before);
        app.select_provider_page(ProviderPage::Openrouter);
        assert_eq!(app.field_value("api_key"), "router-SAVED-PRIVATE");
        app.save_provider_fields();
        assert_eq!(serde_json::to_value(&app.settings).unwrap(), role_settings);
        let saved: AuthFile = serde_json::from_str(
            &std::fs::read_to_string(app.test_config_home.path().join("auth.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            saved.account("grok").unwrap().api_key.as_deref(),
            Some("xai-SAVED-PRIVATE")
        );
        assert_eq!(
            saved.account("openrouter").unwrap().api_key.as_deref(),
            Some("router-SAVED-PRIVATE")
        );
    }

    #[test]
    fn switching_writer_and_tools_never_changes_saved_accounts() {
        let mut app = provider_fixture();
        app.open_module(ModuleId::Providers);
        let before = serde_json::to_value(&app.auth).unwrap();
        for (writer, kind, model) in [
            (true, "openrouter", "vendor/writer"),
            (false, "grok", "grok-tools"),
            (true, "grok", "grok-writer-again"),
        ] {
            app.select_role_provider(writer, kind);
            app.model_target = if writer {
                ModelTarget::Writer
            } else {
                ModelTarget::Tool
            };
            app.select_model(model);
            let runtime = app.role_secret(writer);
            assert_eq!(provider::effective_kind(&runtime), kind);
            assert_eq!(runtime.model, model);
            if kind == "grok" {
                assert_eq!(runtime.kind, "grok-subscription");
                assert!(runtime.api_key.is_none());
            } else {
                assert_eq!(runtime.api_key, app.auth.account(kind).unwrap().api_key);
            }
            assert_eq!(serde_json::to_value(&app.auth).unwrap(), before);
        }
        assert_eq!(app.settings.tool_model, "grok-tools");
        let restarted = App::from_parts(
            Store::memory().unwrap(),
            app.settings.clone(),
            serde_json::from_value(before).unwrap(),
        )
        .unwrap();
        assert!(restarted.role_secret(true).api_key.is_none());
        assert_eq!(
            restarted.auth.account("grok").unwrap().api_key.as_deref(),
            Some("xai-SAVED-PRIVATE")
        );
        assert_eq!(restarted.role_secret(false).model, "grok-tools");
        let config =
            std::fs::read_to_string(app.test_config_home.path().join("config.toml")).unwrap();
        assert!(config.contains("grok-writer-again"));
        assert!(!config.contains("SAVED-PRIVATE"));
    }

    #[test]
    fn grok_subscription_results_are_scoped_and_preserve_credentials() {
        let mut app = provider_fixture();
        let before = serde_json::to_value(&app.auth).unwrap();
        let openai_status = app.subscription_status.clone();
        app.catalog_generation.insert("grok".into(), 2);
        app.grok_subscription_pending = true;
        // Refresh during sign-in must not invalidate its completion callback.
        app.request_catalog(provider::account_secret(&app.auth, "grok"), false);
        assert_eq!(app.catalog_generation["grok"], 2);
        app.on_msg(AppMsg::GrokSubscriptionProgress {
            generation: 1,
            line: "stale".into(),
        });
        assert!(app.grok_subscription_instructions.is_empty());
        app.on_msg(AppMsg::GrokSubscriptionProgress {
            generation: 2,
            line: "device code".into(),
        });
        assert_eq!(app.grok_subscription_instructions, ["device code"]);
        app.on_msg(AppMsg::GrokSubscriptionCheck {
            generation: 2,
            result: Ok(vec![provider::ListedModel {
                id: "grok-verified".into(),
                name: "Grok verified".into(),
                free: false,
            }]),
        });
        assert!(!app.grok_subscription_pending);
        assert!(app.grok_subscription_status.contains("connected"));
        assert_eq!(app.model_catalogs["grok"][0].id, "grok-verified");
        assert_eq!(app.subscription_status, openai_status);
        assert_eq!(serde_json::to_value(&app.auth).unwrap(), before);
        app.on_msg(AppMsg::GrokSubscriptionCheck {
            generation: 1,
            result: Err("stale".into()),
        });
        assert!(app.grok_subscription_status.contains("connected"));
        app.on_msg(AppMsg::GrokSubscriptionCheck {
            generation: 2,
            result: Err("Subscription model access unavailable".into()),
        });
        assert!(!app.model_catalogs.contains_key("grok"));
        assert!(app.grok_subscription_status.contains("unavailable"));
        assert_eq!(serde_json::to_value(&app.auth).unwrap(), before);
        for writer in [true, false] {
            app.select_role_provider(writer, "grok");
            assert_eq!(app.role_secret(writer).kind, "grok-subscription");
            assert!(app.role_secret(writer).api_key.is_none());
        }
    }

    #[test]
    fn openai_setup_is_subscription_only_and_not_a_tool_provider() {
        let mut app = provider_fixture();
        app.select_provider_page(ProviderPage::Openai);
        assert!(!app
            .fields
            .iter()
            .any(|f| f.key == "api_key" || f.key == "base_url"));
        assert!(app.fields.iter().any(|f| f.key == "__subscription_login"));
        assert!(app.role_provider_choices(true).contains(&"openai-chatgpt"));
        assert!(!app.role_provider_choices(true).contains(&"openai"));
        assert!(!app.role_provider_choices(false).contains(&"openai-chatgpt"));
        app.select_role_provider(true, "openai-chatgpt");
        assert_eq!(app.role_secret(true).kind, "openai-chatgpt");
        assert!(app.role_secret(true).api_key.is_none());
        assert_eq!(
            app.role_secret(false).api_key.as_deref(),
            Some("router-SAVED-PRIVATE")
        );
        app.select_role_provider(false, "openai-chatgpt");
        assert_eq!(app.settings.tool_provider, "openrouter");
    }

    #[test]
    fn provider_picker_keyboard_and_section_navigation_are_separate() {
        let mut app = provider_fixture();
        app.open_module(ModuleId::Providers);
        assert_eq!(app.provider_page, ProviderPage::Models);
        app.on_event(key(KeyCode::Enter));
        assert_eq!(app.provider_picker, Some(true));
        app.on_event(key(KeyCode::Down));
        app.on_event(key(KeyCode::Enter));
        assert!(app.provider_picker.is_none());
        assert_eq!(app.settings.writer_provider, "openai-chatgpt");
        assert_eq!(app.settings.tool_provider, "openrouter");
        app.on_event(key(KeyCode::Left));
        assert_eq!(app.provider_page, ProviderPage::Openrouter);
        app.on_event(key(KeyCode::Left));
        assert_eq!(app.provider_page, ProviderPage::Openai);
        app.on_event(key(KeyCode::Right));
        assert_eq!(app.provider_page, ProviderPage::Openrouter);
        app.on_event(key(KeyCode::Esc));
        assert_eq!(app.module, Some(ModuleId::Cases));
    }

    #[test]
    fn model_catalogs_stay_scoped_and_stale_results_are_ignored() {
        let mut app = provider_fixture();
        let model = |id: &str| provider::ListedModel {
            id: id.into(),
            name: id.into(),
            free: false,
        };
        app.catalog_generation.insert("grok".into(), 2);
        app.catalog_generation.insert("openrouter".into(), 1);
        app.on_msg(AppMsg::ModelList {
            kind: "grok".into(),
            generation: 1,
            draft: false,
            result: Ok(vec![model("STALE")]),
        });
        assert!(!app.model_catalogs.contains_key("grok"));
        app.on_msg(AppMsg::ModelList {
            kind: "openrouter".into(),
            generation: 1,
            draft: false,
            result: Ok(vec![model("vendor/router-model")]),
        });
        app.on_msg(AppMsg::ModelList {
            kind: "grok".into(),
            generation: 2,
            draft: false,
            result: Ok(vec![model("grok-account-model")]),
        });
        app.model_target = ModelTarget::Writer;
        assert!(app
            .model_choices()
            .iter()
            .any(|(id, _)| id == "grok-account-model"));
        assert!(!app
            .model_choices()
            .iter()
            .any(|(id, _)| id == "vendor/router-model"));
        app.model_target = ModelTarget::Tool;
        assert!(app
            .model_choices()
            .iter()
            .any(|(id, _)| id == "vendor/router-model"));
        assert!(!app
            .model_choices()
            .iter()
            .any(|(id, _)| id == "grok-account-model"));
        app.catalog_generation.insert("openrouter".into(), 3);
        app.on_msg(AppMsg::ModelList {
            kind: "openrouter".into(),
            generation: 3,
            draft: true,
            result: Ok(vec![model("draft-only")]),
        });
        assert!(!app.model_choices().iter().any(|(id, _)| id == "draft-only"));
        assert!(app.provider_draft_checks["openrouter"].contains("Save to use"));
        assert!(!app.account_status("openrouter").contains("Draft"));
    }

    #[test]
    fn model_query_keeps_letters_and_custom_ids_without_touching_credentials() {
        let mut app = provider_fixture();
        app.model_target = ModelTarget::Writer;
        app.model_picker = true;
        for ch in "grok-new-id".chars() {
            app.on_event(key(KeyCode::Char(ch)));
        }
        assert_eq!(app.model_query, "grok-new-id");
        assert_eq!(app.filtered_model_choices()[0].0, "grok-new-id");
        app.on_event(key(KeyCode::Enter));
        assert_eq!(app.settings.writer_model, "grok-new-id");
        assert_eq!(app.settings.tool_model, "vendor/research");
        assert!(app.role_secret(true).api_key.is_none());
        app.select_provider_page(ProviderPage::Openrouter);
        app.activate_field();
        app.on_event(Event::Key(KeyEvent::new(
            KeyCode::Char('u'),
            KeyModifiers::CONTROL,
        )));
        app.on_event(Event::Paste("router-pasted\n".into()));
        assert_eq!(app.field_value("api_key"), "router-pasted");
    }

    #[test]
    fn grok_recovery_actions_remain_visible_after_failure_on_short_screens() {
        let mut app = provider_fixture();
        app.select_provider_page(ProviderPage::Grok);
        app.grok_subscription_status = "Signed in · Grok model access blocked (spending limit). Check this account's subscription/usage at grok.com, then Check existing login.".into();
        for (width, height) in [(160, 20), (120, 20), (80, 24), (80, 18)] {
            let text = render_workspace(&mut app, width, height);
            for action in [
                "Sign in with Grok",
                "Check existing login",
                "Choose Writer / Tools",
            ] {
                assert!(
                    text.contains(action),
                    "{width}x{height}: Missing {action}\n{text}"
                );
            }
            assert_eq!(app.provider_field_hits.len(), 3);
            println!("Grok recovery {width}x{height}\n{text}");
        }
        app.focus = Focus::Canvas;
        app.on_event(key(KeyCode::Char('j')));
        assert_eq!(app.fields[app.field_sel].key, "__grok_subscription_check");
    }

    #[test]
    fn provider_pages_render_at_normal_and_narrow_sizes_without_secrets_or_mail_setup() {
        let mut app = provider_fixture();
        for (width, height) in [(120, 40), (100, 30), (80, 24), (60, 18)] {
            for page in [
                ProviderPage::Grok,
                ProviderPage::Openai,
                ProviderPage::Openrouter,
                ProviderPage::Models,
            ] {
                app.select_provider_page(page);
                let text = render_workspace(&mut app, width, height);
                assert!(text.contains(page.title()), "{width}x{height}: {text}");
                for unwanted in [
                    "SAVED-PRIVATE",
                    "Mail",
                    "MCP",
                    "OpenAI API key",
                    "text/voice",
                    "LLM",
                ] {
                    assert!(!text.contains(unwanted), "{unwanted} in {text}");
                }
                if matches!((width, height), (120, 40) | (80, 24)) {
                    println!("{page:?} {width}x{height}\n{text}");
                }
            }
        }
        app.select_provider_page(ProviderPage::Openai);
        app.subscription_pending = true;
        app.subscription_instructions = vec![
            "Visit https://auth.openai.com/codex/device".into(),
            "Enter code: TEST-1234".into(),
        ];
        assert!(render_workspace(&mut app, 80, 24).contains("TEST-1234"));
        app.select_provider_page(ProviderPage::Grok);
        app.grok_subscription_pending = true;
        app.grok_subscription_instructions = vec![
            "Opening browser for Grok sign-in".into(),
            "https://auth.x.ai/oauth2/authorize?test=GROK-1234".into(),
        ];
        for (w, h) in [(120, 40), (80, 24)] {
            let text = render_workspace(&mut app, w, h);
            assert!(text.contains("GROK-1234"), "{text}");
            assert!(!text.contains("xAI key"), "{text}");
            println!("Grok browser sign-in {w}x{h}\n{text}");
        }
    }

    #[test]
    fn plus_opens_scope_and_escape_restores_the_query() {
        let store = Store::memory().unwrap();
        let mut app = App::from_parts(store, SettingsFile::default(), AuthFile::default()).unwrap();
        app.prompt = "who is ada".into();
        app.cursor = app.prompt.chars().count();
        app.on_event(key(KeyCode::Char('+')));
        let scope = app.scope.as_ref().expect("scope card");
        assert!(!scope.facts);
        assert!(!scope.domain);
        assert_eq!(scope.query, "who is ada");
        assert!(app.prompt.is_empty());
        assert!(app.pending_reports.is_empty());

        app.on_event(key(KeyCode::Char(' ')));
        assert!(app.scope.as_ref().unwrap().facts);
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
        assert!(text.contains("Space selects"), "{text}");

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
        assert!(app.scope.as_ref().unwrap().web);
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
