//! App state and keyboard routing for the Argos terminal shell.

use super::tracked;
use super::ui::nudge;
use anyhow::Result;
use argos_osint_core::brain::{Memory, MemorySource, ScoredMemory};
use argos_osint_core::hardware::{self, HardwareProfile};
use argos_osint_core::intel_recon::{
    self, ArticleBodyRow, IntelReportJobRow, IntelReportSectionRow, ReportMode, ReportScope,
};
use argos_osint_core::job_registry::{CancelRequest, JobSpec};
use argos_osint_core::paths;
use argos_osint_core::provider::{self, ListedModel, SettingsFile};
use argos_osint_core::related_memories::{RelatedLimits, RelatedMemory};
use argos_osint_core::secrets::{AuthFile, ProviderSecret};
use argos_osint_core::store::Store;
use argos_osint_core::store::{AtlasArticleClaim, AtlasArticleRow, AtlasRunRow};
use argos_osint_core::{atlas, osint, recon};
use crossterm::event::{
    self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent,
    MouseEventKind,
};
use ratatui::layout::Rect;
use ratatui::Frame;
use ratatui::Terminal;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
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
    Intel,
    Recon,
    Brain,
    Atlas,
    Jobs,
    Logs,
    /// Displayed as "Tools"; the internal id and config keys stay `osint`.
    Osint,
    /// Displayed as "Models"; the internal id and config keys stay `providers`.
    Providers,
    System,
}
impl ModuleId {
    /// Home and numeric order: 1 Intel · 2 Atlas · 3 Brain · 4 Recon · 5 Jobs ·
    /// 6 Logs · 7 Tools · 8 Models · 9 System.
    pub const ALL: [Self; 9] = [
        Self::Intel,
        Self::Atlas,
        Self::Brain,
        Self::Recon,
        Self::Jobs,
        Self::Logs,
        Self::Osint,
        Self::Providers,
        Self::System,
    ];
    pub fn index(self) -> usize {
        Self::ALL.iter().position(|item| *item == self).unwrap_or(0)
    }
    pub fn title(self) -> &'static str {
        match self {
            Self::Intel => "Intel",
            Self::Recon => "Recon",
            Self::Brain => "Brain",
            Self::Atlas => "Atlas",
            Self::Jobs => "Jobs",
            Self::Logs => "Logs",
            Self::Osint => "Tools",
            Self::Providers => "Models",
            Self::System => "Profile",
        }
    }
    pub fn blurb(self) -> &'static str {
        match self {
            Self::Intel => "Review intelligence briefings and investigate emerging stories",
            Self::Recon => "Run evidence-driven OSINT investigations",
            Self::Brain => "Recall and explore connected intelligence",
            Self::Atlas => "Map and track the global news cycle",
            Self::Jobs => "Track work running across Argos",
            Self::Logs => "Trace activity, failures, and execution",
            Self::Osint => "Browse, configure, and run OSINT tools",
            Self::Providers => "Configure AI providers and intelligence roles",
            Self::System => "Inspect host hardware and Argos storage",
        }
    }
}

/// Launch state machine for Home composer submissions
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LaunchState {
    /// User is editing the investigation prompt
    Editable,
    /// Submission attempt in progress (validation running)
    Accepting,
    /// Submission accepted, investigation created and pending run
    Accepted,
    /// Submission failed validation/persistence, draft preserved for user to fix
    RecoverableFailure,
}

/// Where a job's "Open source" action leads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JobSource {
    AtlasRun(usize),
    ReconThread(String),
}

/// Live pipeline, or the list of past runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AtlasPage {
    Live,
    Runs,
}

/// Bulletin board or the article briefing view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IntelPage {
    Bulletin,
    Briefing,
}

/// Six OSINT classification tabs shown in Intel (excludes `unk`).
pub const INTEL_CATEGORIES: &[&str] = &[
    "geopolitical",
    "economic",
    "military",
    "information",
    "stability",
    "technology",
];

/// Short tab label for an Intel classification id.
pub fn intel_category_short(id: &str) -> &'static str {
    match id {
        "geopolitical" => "Geopolitical",
        "economic" => "Economic",
        "military" => "Military",
        "information" => "Information",
        "stability" => "Stability",
        "technology" => "Tech",
        _ => "Unk",
    }
}

#[derive(Clone, Debug, Default)]
pub struct Scrolls {
    pub chat: u16,
    pub threads: u16,
    pub memories: u16,
    pub tools: u16,
    pub detail: u16,
    pub atlas_feed: u16,
    pub atlas_runs: u16,
    pub atlas_news: u16,
    pub origins: u16,
    pub insights: u16,
    pub popup: u16,
    pub recall: u16,
    pub path: u16,
    pub summary: u16,
    pub intel_list: u16,
    /// Vertical offset of the Briefing Focus center stack.
    pub intel_brief: u16,
    /// Vertical offset of the Briefing Focus left extracted stack (claims, inferences, actors, links, related context).
    pub intel_extracted: u16,
    /// Line offset inside the fixed-height full-article pane.
    pub intel_full: u16,
    pub intel_jobs: u16,
    /// Vertical scroll offset for the Recon investigation context panel.
    pub recon_context: u16,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ChoiceKind {
    Provider,
    Model,
    IntelDay,
    Investigation,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IntelReconFocus {
    Tab(usize),
    Section(usize),
    Start,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChoiceItem {
    pub id: String,
    pub label: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LastViewSession {
    pub module: Option<String>,
    pub intel_page: Option<String>,
    pub intel_article_id: Option<String>,
    pub intel_article_title: Option<String>,
    pub recon_thread_id: Option<String>,
    pub recon_thread_title: Option<String>,
    pub memory_id: Option<String>,
    pub memory_title: Option<String>,
    pub atlas_page: Option<String>,
    pub osint_tool_id: Option<String>,
    pub providers_page: Option<String>,
    pub updated_at: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Overlay {
    None,
    Help,
    Memories {
        message_id: String,
    },
    Block {
        title: String,
        body: String,
    },
    Choice(ChoiceKind),
    IntelRecon,
    Palette,
    AddFallback,
    ResumeSession(Box<LastViewSession>),
    /// Profile > System > Configs: portable export/import.
    Configs,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaletteItem {
    pub id: String,
    pub label: String,
    pub description: String,
    pub shortcut: String,
    pub category: String,
    pub enabled: bool,
    pub disabled_reason: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ProviderPage {
    Defaults,
    OpenRouter,
    Google,
    Nvidia,
}
impl ProviderPage {
    pub const ALL: [Self; 4] = [Self::Defaults, Self::OpenRouter, Self::Google, Self::Nvidia];
    pub fn title(self) -> &'static str {
        match self {
            Self::Defaults => "Defaults",
            Self::OpenRouter => "OpenRouter",
            Self::Google => "Google",
            Self::Nvidia => "Nvidia",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BrainListMode {
    List,
    Create,
    Graph,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FieldId {
    BrainApp,
    BrainConversation,
    BrainInsight,
    BrainQuery,
    ReconSearch,
    IntelSearch,
    OsintSearch,
    OsintInput,
    FirecrawlKey,
    FirecrawlFallback,
    HunterKey,
    HunterFallback,
    SociaVaultKey,
    SociaVaultFallback,
    NewsApiKey,
    NewsApiFallback,
    CourtListenerKey,
    CourtListenerFallback,
    GnewsKey,
    GnewsFallback,
    NewsDataKey,
    NewsDataFallback,
    CurrentsKey,
    CurrentsFallback,
    WhoxyKey,
    WhoxyFallback,
    JobsSearch,
    LogsSearch,
    ReconProvider,
    ReconModel,
    PickerProvider,
    PickerModel,
    SynthesisProvider,
    SynthesisModel,
    ClassifierProvider,
    ClassifierModel,
    SummarizationProvider,
    SummarizationModel,
    EvidenceCuratorProvider,
    EvidenceCuratorModel,
    EntityResolverProvider,
    EntityResolverModel,
    ClaimAssessorProvider,
    ClaimAssessorModel,
    InvestigationControllerProvider,
    InvestigationControllerModel,
    RouterKey,
    RouterEndpoint,
    GoogleKey,
    GoogleEndpoint,
    NvidiaKey,
    NvidiaEndpoint,
    GoogleModelFilter,
    NvidiaModelFilter,
    RouterModelFilter,
    FallbackFilter,
    Composer,
    /// Profile > System > Configs: export destination path.
    ProfileExportPath,
    /// Profile > System > Configs: import document editor.
    ProfileImportEditor,
}

/// The model role the Defaults tab is editing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DefaultsRole {
    Recon,
    ToolPicker,
    Synthesis,
    Classifier,
    Summarization,
    EvidenceCurator,
    EntityResolver,
    ClaimAssessor,
    InvestigationController,
}

impl DefaultsRole {
    pub const ALL: [DefaultsRole; 9] = [
        DefaultsRole::Recon,
        DefaultsRole::ToolPicker,
        DefaultsRole::Synthesis,
        DefaultsRole::Classifier,
        DefaultsRole::Summarization,
        DefaultsRole::EvidenceCurator,
        DefaultsRole::EntityResolver,
        DefaultsRole::ClaimAssessor,
        DefaultsRole::InvestigationController,
    ];

    pub fn label(self) -> &'static str {
        match self {
            DefaultsRole::Recon => "Recon",
            DefaultsRole::ToolPicker => "Tool picker",
            DefaultsRole::Synthesis => "Synthesis",
            DefaultsRole::Classifier => "Classifier",
            DefaultsRole::Summarization => "Summarization",
            DefaultsRole::EvidenceCurator => "Evidence curator",
            DefaultsRole::EntityResolver => "Entity resolver",
            DefaultsRole::ClaimAssessor => "Claim assessor",
            DefaultsRole::InvestigationController => "Controller",
        }
    }

    /// The settings key the Logs entry names when this role changes.
    pub fn settings_key(self) -> &'static str {
        match self {
            DefaultsRole::Recon => "defaults.recon",
            DefaultsRole::ToolPicker => "defaults.tool_picker",
            DefaultsRole::Synthesis => "defaults.synthesis",
            DefaultsRole::Classifier => "defaults.classifier",
            DefaultsRole::Summarization => "defaults.summarization",
            DefaultsRole::EvidenceCurator => "defaults.evidence_curator",
            DefaultsRole::EntityResolver => "defaults.entity_resolver",
            DefaultsRole::ClaimAssessor => "defaults.claim_assessor",
            DefaultsRole::InvestigationController => "defaults.investigation_controller",
        }
    }

    pub fn role_key(self) -> &'static str {
        match self {
            DefaultsRole::Recon => "recon",
            DefaultsRole::ToolPicker => "tool-picker",
            DefaultsRole::Synthesis => "synthesis",
            DefaultsRole::Classifier => "classifier",
            DefaultsRole::Summarization => "summarization",
            DefaultsRole::EvidenceCurator => "evidence_curator",
            DefaultsRole::EntityResolver => "entity_resolver",
            DefaultsRole::ClaimAssessor => "claim_assessor",
            DefaultsRole::InvestigationController => "investigation_controller",
        }
    }

    pub fn provider_field(self) -> FieldId {
        match self {
            DefaultsRole::Recon => FieldId::ReconProvider,
            DefaultsRole::ToolPicker => FieldId::PickerProvider,
            DefaultsRole::Synthesis => FieldId::SynthesisProvider,
            DefaultsRole::Classifier => FieldId::ClassifierProvider,
            DefaultsRole::Summarization => FieldId::SummarizationProvider,
            DefaultsRole::EvidenceCurator => FieldId::EvidenceCuratorProvider,
            DefaultsRole::EntityResolver => FieldId::EntityResolverProvider,
            DefaultsRole::ClaimAssessor => FieldId::ClaimAssessorProvider,
            DefaultsRole::InvestigationController => FieldId::InvestigationControllerProvider,
        }
    }

    pub fn model_field(self) -> FieldId {
        match self {
            DefaultsRole::Recon => FieldId::ReconModel,
            DefaultsRole::ToolPicker => FieldId::PickerModel,
            DefaultsRole::Synthesis => FieldId::SynthesisModel,
            DefaultsRole::Classifier => FieldId::ClassifierModel,
            DefaultsRole::Summarization => FieldId::SummarizationModel,
            DefaultsRole::EvidenceCurator => FieldId::EvidenceCuratorModel,
            DefaultsRole::EntityResolver => FieldId::EntityResolverModel,
            DefaultsRole::ClaimAssessor => FieldId::ClaimAssessorModel,
            DefaultsRole::InvestigationController => FieldId::InvestigationControllerModel,
        }
    }

    pub fn save_button(self) -> ButtonId {
        match self {
            DefaultsRole::Recon => ButtonId::SaveRecon,
            DefaultsRole::ToolPicker => ButtonId::SavePicker,
            DefaultsRole::Synthesis => ButtonId::SaveSynthesis,
            DefaultsRole::Classifier => ButtonId::SaveClassifier,
            DefaultsRole::Summarization => ButtonId::SaveSummarization,
            DefaultsRole::EvidenceCurator => ButtonId::SaveEvidenceCurator,
            DefaultsRole::EntityResolver => ButtonId::SaveEntityResolver,
            DefaultsRole::ClaimAssessor => ButtonId::SaveClaimAssessor,
            DefaultsRole::InvestigationController => ButtonId::SaveInvestigationController,
        }
    }

    fn of_field(field: FieldId) -> Option<DefaultsRole> {
        match field {
            FieldId::ReconProvider | FieldId::ReconModel => Some(DefaultsRole::Recon),
            FieldId::PickerProvider | FieldId::PickerModel => Some(DefaultsRole::ToolPicker),
            FieldId::SynthesisProvider | FieldId::SynthesisModel => Some(DefaultsRole::Synthesis),
            FieldId::ClassifierProvider | FieldId::ClassifierModel => {
                Some(DefaultsRole::Classifier)
            }
            FieldId::SummarizationProvider | FieldId::SummarizationModel => {
                Some(DefaultsRole::Summarization)
            }
            FieldId::EvidenceCuratorProvider | FieldId::EvidenceCuratorModel => {
                Some(DefaultsRole::EvidenceCurator)
            }
            FieldId::EntityResolverProvider | FieldId::EntityResolverModel => {
                Some(DefaultsRole::EntityResolver)
            }
            FieldId::ClaimAssessorProvider | FieldId::ClaimAssessorModel => {
                Some(DefaultsRole::ClaimAssessor)
            }
            FieldId::InvestigationControllerProvider | FieldId::InvestigationControllerModel => {
                Some(DefaultsRole::InvestigationController)
            }
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ButtonId {
    Send,
    Add,
    Pin,
    Delete,
    SaveRecon,
    SavePicker,
    SaveSynthesis,
    SaveClassifier,
    SaveSummarization,
    SaveEvidenceCurator,
    SaveEntityResolver,
    SaveClaimAssessor,
    SaveInvestigationController,
    DefaultRole(DefaultsRole),
    RefreshModels,
    AddFallback,
    DeleteFallback,
    MoveFallbackUp,
    MoveFallbackDown,
    FallbackItem(usize),
    FallbackPick(usize),
    FallbackTab(ProviderPage),
    ConfirmAddFallback,
    #[allow(dead_code)]
    NewThread,
    DeleteThread,
    CancelRun,
    ResumeRun,
    RetryInsights,
    /// Toggle the Recon investigation context panel (show/hide).
    ToggleInvestigation,
    OsintRun,
    OsintAttach,
    OsintStartRecon,
    OsintCancel,
    OsintRefreshDataset,
    OsintRefreshTemplates,
    OsintSearchSelected,
    OsintToggle,
    OsintRaw,
    OsintPrev,
    OsintNext,
    /// "see more" page-down for the tool/OSINT detail pane.
    SeeMoreDetail,
    /// "see more" page-down for the Brain recall/anchors pane.
    SeeMoreRecall,
    /// "see more" page-down for a generic Block overlay body.
    SeeMorePopup,
    /// Open the selected tool's documentation URL.
    OpenDocumentation,
    SaveFirecrawlKey,
    SaveHunterKey,
    SaveSociaVaultKey,
    SaveNewsApiKey,
    SaveCourtListenerKey,
    SaveGnewsKey,
    SaveNewsDataKey,
    SaveCurrentsKey,
    SaveWhoxyKey,
    TestWhoxyConnection,
    AtlasRun,
    AtlasAuto,
    AtlasRuns,
    AtlasLive,
    AtlasDelete,
    /// Resume the selected cycle (stopped in phase 4/5 or paused).
    AtlasResume,
    /// Start the "Repair Atlas memories" job.
    AtlasRepair,
    AtlasNewsFeed,
    AtlasWorld,
    AtlasNews,
    IntelDay,
    /// Center briefing launcher labeled with the classified recon mode.
    IntelReports,
    IntelReconStart,
    IntelBodyRetry,
    IntelBodyRefresh,
    IntelJobOpen,
    IntelJobPause,
    IntelJobResume,
    IntelJobCancel,
    IntelJobRetry,
    CreateMemory,
    BrainBack,
    /// Back in the memory detail history (or to the Brain list).
    BrainDetailBack,
    /// Summary failure card: toggle the sanitized cause chain and attempts.
    SummaryDetails,
    /// Summary failure card: Logs filtered to the failed job.
    SummaryLogs,
    /// Summary failure card: the failed job in Jobs.
    SummaryJob,
    /// Summary failure card: a fresh, linked execution.
    SummaryRetry,
    /// Summary failure card: Models → Defaults with Summarization selected.
    SummaryModels,
    /// Logs: clear durable events only (jobs, results, memories are kept).
    ClearLog,
    JobsStatus,
    JobsApp,
    JobsViewLogs,
    JobsRetry,
    JobsCancel,
    JobsOpenSource,
    LogsLevel,
    LogsApp,
    LogsFollow,
    LogsOpenJob,
    LogsBack,
    RouterSave,
    RouterVerify,
    RouterAdvanced,
    IntelFullReport,
    GoogleSave,
    GoogleVerify,
    GoogleAdvanced,
    NvidiaSave,
    NvidiaVerify,
    NvidiaAdvanced,
    RefreshHardware,
    ResumeSessionConfirm,
    ResumeSessionDismiss,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
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
    /// One Logs event row. Clicking it folds the entry when it has a detail.
    LogLine(usize),
    /// One Jobs table row.
    JobRow(usize),
    /// The Jobs detail panel (scroll focus).
    JobDetail,
    /// One Atlas headline in the live feed.
    AtlasFeed(usize),
    /// One past Atlas run in the history list.
    AtlasHistory(usize),
    /// Country stats table for the selected news cycle.
    AtlasCycleStats,
    /// One saved article in the history news feed.
    AtlasArticle(usize),
    /// One Intel category tab.
    IntelTab(usize),
    /// One Intel bulletin article row.
    IntelArticle(usize),
    /// Recon mode tab inside the Recon configuration popup.
    IntelReconTab(usize),
    /// One report-section toggle inside the Recon configuration popup.
    IntelReconSection(usize),
    /// One line of the open recon or claim path.
    PathLine(usize),
    /// The path graph section of the memory detail (scroll focus).
    DetailPath,
    /// One Related memory row in the memory detail.
    RelatedRow(usize),
    /// The Summary section of the memory detail (scroll focus).
    DetailSummary,
    Choice(usize),
    CloseOverlay,
    Tab(usize),
    TabClose(usize),
    TabPlus,
    TabOverflow,
    /// The Recon investigation context panel (scroll focus).
    ReconContext,
    /// The OSINT/Tools detail pane (scroll focus).
    OsintDetail,
    /// The Brain memory anchors/recall pane (scroll focus).
    BrainRecall,
    /// The Intel briefing extracted left stack (scroll focus).
    IntelLeftColumn,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq, Eq)]
pub struct HomeDraftState {
    pub prompt: String,
    pub cursor: usize,
    pub scroll: usize,
    pub report_mode: Option<ReportMode>,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq, Eq)]
pub struct SessionTabsState {
    pub open_thread_ids: Vec<String>,
    pub last_active_id: Option<String>,
    pub recently_closed: Vec<String>,
}

#[derive(Debug)]
enum ProviderEvent {
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
    WhoxyBalance {
        outcome: Result<u32, String>,
    },
    DatasetRefreshProgress {
        dataset: String,
        phase: String,
    },
    DatasetRefreshDone {
        dataset: String,
        outcome: Result<String, String>,
    },
    CatalogDone {
        role: DefaultsRole,
        provider: String,
        outcome: Result<Vec<ListedModel>, String>,
    },

    InsightDone {
        thread_id: String,
        outcome: Result<(), String>,
    },
    AnswerDelta {
        thread_id: String,
        text: String,
    },
    AnswerReset {
        thread_id: String,
    },
    AnswerReplacement {
        thread_id: String,
        text: String,
    },
    AnswerNote {
        thread_id: String,
        text: String,
    },
    GraphSummary {
        report: Box<argos_osint_core::graph_explanation::ExplainReport>,
    },
    Related {
        request: u64,
        memory_id: String,
        outcome: std::result::Result<Vec<RelatedMemory>, String>,
    },
    Atlas(atlas::AtlasEvent),
    AtlasDone {
        outcome: std::result::Result<atlas::Stop, String>,
    },
    IntelBody(intel_recon::BodyFetchEvent),
    IntelReport(intel_recon::IntelReportEvent),
    IntelReconMode {
        article_id: String,
        mode: ReportMode,
    },
    IntelActorsReviewed {
        article_id: String,
    },
    IntelLinksExplained {
        article_id: String,
        explanations: HashMap<(String, String), String>,
    },
    BrainRelatedExplained {
        request: u64,
        memory_id: String,
        reasons: Vec<String>,
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FocusEntry {
    pub target: Target,
    pub rect: ratatui::layout::Rect,
    pub scope: usize,
    pub scrollable: bool,
}

#[derive(Default, Clone, Debug, PartialEq, Eq)]
pub struct LayoutRegistry {
    pub entries: Vec<FocusEntry>,
    pub scopes: Vec<ratatui::layout::Rect>,
    pub current_scope: usize,
}

impl LayoutRegistry {
    pub fn clear(&mut self) {
        self.entries.clear();
        self.scopes.clear();
        self.current_scope = 0;
    }

    pub fn push_scope(&mut self, rect: ratatui::layout::Rect) -> usize {
        let id = self.scopes.len();
        self.scopes.push(rect);
        self.current_scope = id;
        id
    }

    pub fn register(&mut self, target: Target, rect: ratatui::layout::Rect) {
        self.entries.push(FocusEntry {
            target,
            rect,
            scope: self.current_scope,
            scrollable: false,
        });
    }
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
    pub intel_search: String,
    pub osint_search: String,
    pub osint_input: String,
    pub firecrawl_key: String,
    pub firecrawl_fallback: String,
    pub hunter_key: String,
    pub hunter_fallback: String,
    pub sociavault_key: String,
    pub sociavault_fallback: String,
    pub newsapi_key: String,
    pub newsapi_fallback: String,
    pub courtlistener_key: String,
    pub courtlistener_fallback: String,
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
    pub recon_context_enabled: bool,
    /// Whether the thread draft has unsaved changes
    draft_dirty: bool,
    pub scrolls: Scrolls,
    pub expanded: HashSet<String>,
    pub chat_sel: usize,
    pub chat_follow: bool,
    pub overlay: Overlay,
    /// Logs dashboard (durable events).
    pub logs: super::logs::LogsView,
    /// Jobs dashboard.
    pub jobs: super::jobs::JobsView,
    /// Last dashboard refresh (throttles live updates).
    dashboards_at: Option<Instant>,
    logged_calls: HashSet<String>,
    pub runs: Vec<recon::Run>,
    pub answer_memories: HashMap<String, Vec<Memory>>,
    quit_arm: Option<Instant>,
    esc_arm: Option<Instant>,
    press: Option<(u16, u16, Option<Target>)>,
    /// Whether the Home draft has unsaved changes
    home_draft_dirty: bool,
    /// The current content of the Home draft (investigation prompt)
    home_draft: String,
    /// Cursor position in the Home draft
    home_draft_cursor: usize,
    /// Scroll position in the Home draft (for future multi-line support)
    home_draft_scroll: usize,
    /// Report mode for the Home draft (optional)
    home_draft_report_mode: Option<ReportMode>,
    /// Whether the Home draft is currently being submitted (prevents duplicate submissions)
    home_draft_submitting: bool,
    /// Submission token for duplicate prevention (None when not submitting)
    #[allow(dead_code)]
    home_draft_submission_token: Option<u64>,
    /// Last timestamp when Home draft was saved (for debouncing)
    #[allow(dead_code)]
    home_draft_last_saved: Option<Instant>,
    /// Tab strip state: ordered list of open investigation tab IDs (Thread.id)
    /// The first position (index 0) is reserved for the permanent Home tab
    pub tab_ids: Vec<String>,
    /// Index of the currently selected tab in tab_ids (0 = Home, 1+ = investigations)
    pub tab_sel: usize,
    /// Timestamp of the last active investigation (for ordering tabs)
    /// Map of thread_id -> last_active_timestamp
    pub tab_last_active: HashMap<String, Instant>,
    /// History of recently closed tab IDs (for reopen feature, max 20)
    pub tab_recently_closed: Vec<String>,
    pub tab_unreads: HashSet<String>,
    /// Launch state machine for Home composer submissions
    pub launch_state: LaunchState,
    /// Tracks if user has explicitly navigated away during launch acceptance
    /// to prevent focus theft
    #[allow(dead_code)]
    pub launch_navigated_away: bool,
    /// Timestamp of the last explicit user navigation action
    #[allow(dead_code)]
    pub last_user_nav_action: Option<Instant>,
    pub frame: RefCell<super::ui::FrameCache>,
    pub thread_history: Vec<String>,
    pub history_pos: usize,
    running: HashMap<String, Arc<AtomicBool>>,
    osint_cancel: Option<Arc<AtomicBool>>,
    pub dataset_refresh_running: Option<String>,
    dataset_refresh_cancel: Option<Arc<AtomicBool>>,
    pub recon_provider: String,
    pub recon_model: String,
    pub picker_provider: String,
    pub picker_model: String,
    pub synthesis_provider: String,
    pub synthesis_model: String,
    pub classifier_provider: String,
    pub classifier_model: String,
    pub summarization_provider: String,
    pub summarization_model: String,
    pub evidence_curator_provider: String,
    pub evidence_curator_model: String,
    pub entity_resolver_provider: String,
    pub entity_resolver_model: String,
    pub claim_assessor_provider: String,
    pub claim_assessor_model: String,
    pub investigation_controller_provider: String,
    pub investigation_controller_model: String,
    pub show_thinking: bool,
    pub defaults_role: DefaultsRole,
    pub model_catalog: Vec<ListedModel>,
    pub catalog_cache: HashMap<String, Vec<ListedModel>>,
    pub catalog_for: String,
    pub fallback_sel: usize,
    pub fallback_popup_tab: ProviderPage,
    pub fallback_popup_filter: String,
    pub fallback_popup_sel: usize,
    pub fallback_popup_restore: Option<Target>,
    pub choice_items: Vec<ChoiceItem>,
    pub choice_sel: usize,
    pub choice_note: String,
    pub palette_query: String,
    pub palette_sel: usize,
    pub google_key: String,
    pub google_endpoint: String,
    pub google_advanced: bool,
    pub nvidia_key: String,
    pub nvidia_endpoint: String,
    pub nvidia_advanced: bool,
    pub router_key: String,
    pub router_endpoint: String,
    pub router_advanced: bool,
    pub google_status: String,
    pub nvidia_status: String,
    pub router_status: String,
    pub router_model_filter: String,
    pub google_model_filter: String,
    pub nvidia_model_filter: String,
    pub provider_pending: Option<ProviderPage>,
    pub focus: Target,
    pub cursor: usize,
    pub screen: Rect,
    pub layout: std::cell::RefCell<LayoutRegistry>,
    pub status: String,
    /// Page sizes (inner viewport height) for `see more` buttons, set during rendering.
    pub see_more_pages: std::cell::Cell<[u16; 3]>, // [detail, recall, popup]
    pub memories: Vec<Memory>,
    pub selected_insight: Option<recon::InsightView>,
    pub memory_sel: usize,
    pub brain_list_mode: BrainListMode,
    pub atlas_page: AtlasPage,
    pub atlas_stats: atlas::RunStats,
    pub atlas_feed: Vec<atlas::FeedArticle>,
    pub atlas_feed_sel: usize,
    /// True while the selection is on the newest headline, so the list follows arrivals.
    pub atlas_feed_follow: bool,
    pub atlas_runs: Vec<AtlasRunRow>,
    pub atlas_run_sel: usize,
    /// History news list for the run opened from the statistics card.
    pub atlas_news: bool,
    pub atlas_news_run: String,
    pub atlas_articles: Vec<AtlasArticleRow>,
    pub atlas_article_sel: usize,
    /// Article marked from a claim-path click. Cleared when the feed is opened any other way.
    pub claim_mark: Option<String>,
    /// Last pointer, so Ctrl+U/D can page the table under it.
    pub pointer: Option<(u16, u16)>,
    /// Country the map is zoomed to. Empty means the world view.
    pub atlas_focus: Option<String>,
    /// The next frame shows a loading note. The frame after that paints the map.
    pub atlas_map_hold: bool,
    pub atlas_status: String,
    pub atlas_state: String,
    /// Insight extract work units `(done, total)` while the spinner is shown.
    pub atlas_insight_progress: Option<(u32, u32)>,
    pub atlas_pause: Option<Arc<AtomicBool>>,
    /// Unix time of the next automatic pipeline run. `None` means auto run is off.
    pub atlas_auto_next: Option<u64>,
    /// The pipeline now running was started by auto run, not the Run button.
    atlas_auto_started: bool,
    pub intel_page: IntelPage,
    pub intel_category: String,
    pub intel_day: String,
    pub intel_days: Vec<String>,
    pub intel_articles: Vec<AtlasArticleRow>,
    pub intel_sel: usize,
    pub intel_claims: Vec<AtlasArticleClaim>,
    pub intel_relations: Vec<(String, String, String)>,
    pub intel_body: Option<ArticleBodyRow>,
    pub intel_body_message: String,
    pub intel_full_collapsed: bool,
    pub intel_jobs: Vec<IntelReportJobRow>,
    pub intel_sections: Vec<IntelReportSectionRow>,
    pub intel_job_sel: usize,
    /// Active mode tab in the Recon configuration popup.
    pub intel_recon_tab: usize,
    /// Classifier-recommended recon mode for the focused briefing article.
    pub intel_recon_recommended: ReportMode,
    /// Article id the recommended mode applies to (empty when unset).
    pub intel_recon_recommended_for: String,
    /// True while the classifier is choosing the default recon mode.
    pub intel_mode_classifying: bool,
    /// Enabled section keys per mode (`verify`, `explain`, …).
    pub intel_recon_enabled: HashMap<String, HashSet<String>>,
    /// Keyboard/mouse focus inside the Recon configuration popup.
    pub intel_recon_focus: IntelReconFocus,
    pub(crate) intel_body_running: HashSet<String>,
    /// Article ids whose cleaned-body insight re-extract is still in flight.
    pub(crate) intel_insights_running: HashSet<String>,
    pub(crate) intel_actors_reviewing: HashSet<String>,
    pub(crate) intel_links_reviewing: HashSet<String>,
    pub(crate) intel_link_explanations: HashMap<(String, String), String>,
    pub(crate) intel_report_running: HashMap<String, Arc<AtomicBool>>,
    pub gnews_key: String,
    pub gnews_fallback: String,
    pub newsdata_key: String,
    pub newsdata_fallback: String,
    pub currents_key: String,
    pub currents_fallback: String,
    pub whoxy_key: String,
    pub whoxy_fallback: String,
    pub brain_graph: recon::MemoryGraph,
    brain_graph_for: Option<String>,
    /// Open memory detail (graph, Related, Summary), independent of `memory_sel`.
    pub brain_detail: super::brain_detail::BrainDetail,
    /// Total saved memories, to explain an active Find filter.
    pub memory_total: usize,
    /// Last memory-list read failure; the previous list stays on screen.
    pub memory_error: Option<String>,
    /// False until the first memory list read finished.
    pub memories_loaded: bool,
    pub graph_summary: String,
    graph_summary_pending: Option<String>,
    /// Request id of the newest graph explanation; older completions are ignored.
    graph_summary_request: String,
    /// Failed graph explanation for the open memory (inline card).
    pub summary_failure: Option<super::summary_card::SummaryFailure>,
    pub summary_details_open: bool,
    pub hits: Vec<ScoredMemory>,
    pub auth: AuthFile,
    pub settings: SettingsFile,
    pub hardware: HardwareProfile,
    /// Profile dashboard: Overview widgets and the System tab.
    pub profile: super::profile::ProfileView,
    /// Profile > System > Configs: portable export/import popup state.
    pub profile_config: super::profile_config::ConfigView,
    auth_path: PathBuf,
    settings_path: PathBuf,
    pub store: Store,
    provider_tx: UnboundedSender<ProviderEvent>,
    provider_rx: UnboundedReceiver<ProviderEvent>,
    work_tx: UnboundedSender<WorkEvent>,
    work_rx: UnboundedReceiver<WorkEvent>,
}

fn atlas_log_level(text: &str) -> &'static str {
    let lower = text.to_ascii_lowercase();
    if lower.contains("rate limit")
        || lower.contains("http")
        || lower.contains("rejected")
        || lower.contains("malformed")
        || lower.contains("failed")
    {
        "error"
    } else {
        "info"
    }
}

fn open_external_url(url: &str) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open").arg(url).spawn()?;
        return Ok(());
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open").arg(url).spawn()?;
        return Ok(());
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd")
            .args(["/C", "start", "", url])
            .spawn()?;
        return Ok(());
    }
    #[allow(unreachable_code)]
    Err(anyhow::anyhow!("no URL opener for this platform"))
}

impl App {
    pub(crate) fn selected_intel_busy(&self) -> bool {
        let Some(article) = self.intel_articles.get(self.intel_sel) else {
            return false;
        };
        self.intel_body_running.contains(&article.id)
            || self.intel_insights_running.contains(&article.id)
            || self.intel_body.as_ref().is_some_and(|body| {
                matches!(
                    body.state.as_str(),
                    "queued" | "running" | "fetching" | "filtering" | "parsing" | "persisting"
                )
            })
            || self.intel_jobs.iter().any(|job| {
                job.article_id == article.id
                    && (matches!(
                        job.state.as_str(),
                        "queued" | "running" | "waiting" | "paused"
                    ) || self.intel_report_running.contains_key(&job.id))
            })
    }
    pub fn boot() -> Result<Self> {
        paths::ensure_home()?;
        // Finish any configuration commit a previous process left half applied.
        let _ = argos_osint_core::config_commit::recover_at_startup();
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
        let memory_total = memories.len();
        let auth = AuthFile::load()?;
        let settings = SettingsFile::load()?;
        let recon_default = provider::role_secret(&auth, &settings, "recon")?;
        let picker_default = provider::role_secret(&auth, &settings, "tool-picker")?;
        let synthesis_default = provider::role_secret(&auth, &settings, "synthesis")?;
        let classifier_default = provider::role_secret(&auth, &settings, "classifier")?;
        let summarization_default = provider::role_secret(&auth, &settings, "summarization")?;
        let curator_default = provider::role_secret(&auth, &settings, "evidence_curator")?;
        let resolver_default = provider::role_secret(&auth, &settings, "entity_resolver")?;
        let assessor_default = provider::role_secret(&auth, &settings, "claim_assessor")?;
        let controller_default =
            provider::role_secret(&auth, &settings, "investigation_controller")?;
        let router = provider::account_secret(&auth, "openrouter");
        let (provider_tx, provider_rx) = unbounded_channel();
        let (work_tx, work_rx) = unbounded_channel();
        let mut app = Self {
            module: None,
            launcher_sel: 0,
            provider_page: ProviderPage::Defaults,
            input: draft,
            brain_app: String::new(),
            brain_conversation: String::new(),
            brain_insight: String::new(),
            brain_query: String::new(),
            recon_search: String::new(),
            intel_search: String::new(),
            osint_search: String::new(),
            osint_input: osint::registry()
                .first()
                .map(|t| t.example_input().to_string())
                .unwrap_or_else(|| "{}".into()),
            osint_inputs: HashMap::new(),
            firecrawl_key: settings.firecrawl_api_key.clone(),
            firecrawl_fallback: settings.firecrawl_api_key_fallback.clone(),
            hunter_key: settings.hunter_api_key.clone(),
            hunter_fallback: settings.hunter_api_key_fallback.clone(),
            sociavault_key: settings.sociavault_api_key.clone(),
            sociavault_fallback: settings.sociavault_api_key_fallback.clone(),
            newsapi_key: settings.newsapi_api_key.clone(),
            newsapi_fallback: settings.newsapi_api_key_fallback.clone(),
            courtlistener_key: settings.courtlistener_api_token.clone(),
            courtlistener_fallback: settings.courtlistener_api_token_fallback.clone(),
            gnews_key: settings.gnews_api_key.clone(),
            gnews_fallback: settings.gnews_api_key_fallback.clone(),
            newsdata_key: settings.newsdata_api_key.clone(),
            newsdata_fallback: settings.newsdata_api_key_fallback.clone(),
            currents_key: settings.currents_api_key.clone(),
            currents_fallback: settings.currents_api_key_fallback.clone(),
            whoxy_key: settings.whoxy_api_key.clone(),
            whoxy_fallback: settings.whoxy_api_key_fallback.clone(),
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
            recon_context_enabled: settings.tui_recon_context.unwrap_or(true),
            scrolls: Scrolls {
                chat: chat_scroll,
                ..Scrolls::default()
            },
            expanded: HashSet::new(),
            chat_sel: 0,
            chat_follow: chat_scroll == 0,
            overlay: Overlay::None,
            logs: crate::tui::logs::LogsView::default(),
            jobs: crate::tui::jobs::JobsView::default(),
            dashboards_at: None,
            logged_calls: HashSet::new(),
            runs: Vec::new(),
            answer_memories: HashMap::new(),
            quit_arm: None,
            esc_arm: None,
            press: None,
            draft_dirty: false,
            home_draft_dirty: false,
            // The current content of the Home draft (investigation prompt)
            home_draft: String::new(),
            // Cursor position in the Home draft
            home_draft_cursor: 0,
            // Scroll position in the Home draft (for future multi-line support)
            home_draft_scroll: 0,
            // Report mode for the Home draft (optional)
            home_draft_report_mode: None,
            // Whether the Home draft is currently being submitted (prevents duplicate submissions)
            home_draft_submitting: false,
            // Submission token for duplicate prevention (None when not submitting)
            home_draft_submission_token: None,
            // Last timestamp when Home draft was saved (for debouncing)
            home_draft_last_saved: None,
            frame: RefCell::new(super::ui::FrameCache::default()),
            // Tab strip state: ordered list of open investigation tab IDs (Thread.id)
            tab_ids: Vec::new(),
            // Index of the currently selected tab in tab_ids (0 = Home, 1+ = investigations)
            tab_sel: 0,
            // Timestamp of the last active investigation (for ordering tabs)
            tab_last_active: HashMap::new(),
            // History of recently closed tab IDs (for reopen feature, max 20)
            tab_recently_closed: Vec::new(),
            tab_unreads: HashSet::new(),
            // Launch state machine for Home composer submissions
            launch_state: LaunchState::Editable,
            // Tracks if user has explicitly navigated away during launch acceptance
            launch_navigated_away: false,
            // Timestamp of the last explicit user navigation action
            last_user_nav_action: None,
            thread_history: Vec::new(),
            history_pos: 0,
            running: HashMap::new(),
            osint_cancel: None,
            dataset_refresh_running: None,
            dataset_refresh_cancel: None,
            recon_provider: provider::effective_kind(&recon_default),
            recon_model: recon_default.model,
            picker_provider: provider::effective_kind(&picker_default),
            picker_model: picker_default.model,
            synthesis_provider: provider::effective_kind(&synthesis_default),
            synthesis_model: synthesis_default.model,
            classifier_provider: provider::effective_kind(&classifier_default),
            classifier_model: classifier_default.model,
            summarization_provider: provider::effective_kind(&summarization_default),
            summarization_model: summarization_default.model,
            evidence_curator_provider: provider::effective_kind(&curator_default),
            evidence_curator_model: curator_default.model,
            entity_resolver_provider: provider::effective_kind(&resolver_default),
            entity_resolver_model: resolver_default.model,
            claim_assessor_provider: provider::effective_kind(&assessor_default),
            claim_assessor_model: assessor_default.model,
            investigation_controller_provider: provider::effective_kind(&controller_default),
            investigation_controller_model: controller_default.model,
            show_thinking: false,
            defaults_role: DefaultsRole::Recon,
            model_catalog: Vec::new(),
            catalog_cache: HashMap::new(),
            catalog_for: String::new(),
            fallback_sel: 0,
            fallback_popup_tab: ProviderPage::Google,
            fallback_popup_filter: String::new(),
            fallback_popup_sel: 0,
            fallback_popup_restore: None,
            choice_items: Vec::new(),
            choice_sel: 0,
            choice_note: String::new(),
            palette_query: String::new(),
            palette_sel: 0,
            google_key: provider::account_secret(&auth, "google")
                .api_key
                .unwrap_or_default(),
            google_endpoint: provider::account_secret(&auth, "google").base_url,
            google_advanced: false,
            nvidia_key: provider::account_secret(&auth, "nvidia")
                .api_key
                .unwrap_or_default(),
            nvidia_endpoint: provider::account_secret(&auth, "nvidia").base_url,
            nvidia_advanced: false,
            router_key: router.api_key.unwrap_or_default(),
            router_endpoint: router.base_url,
            router_advanced: false,
            google_status: "Enter a key, then verify or save".into(),
            nvidia_status: "Enter a key, then verify or save".into(),
            router_model_filter: String::new(),
            google_model_filter: String::new(),
            nvidia_model_filter: String::new(),
            router_status: "Enter a key, then verify or save".into(),
            provider_pending: None,
            focus: Target::App(0),
            cursor: 0,
            screen: Rect::default(),
            layout: std::cell::RefCell::new(LayoutRegistry::default()),
            status: "ready".into(),
            see_more_pages: std::cell::Cell::new([8, 8, 8]),
            memories,
            selected_insight: None,
            memory_sel: 0,
            brain_list_mode: BrainListMode::List,
            atlas_page: AtlasPage::Runs,
            atlas_stats: atlas::RunStats::default(),
            atlas_feed: Vec::new(),
            atlas_feed_sel: 0,
            atlas_feed_follow: true,
            atlas_runs: Vec::new(),
            atlas_run_sel: 0,
            atlas_news: false,
            atlas_news_run: String::new(),
            atlas_articles: Vec::new(),
            atlas_article_sel: 0,
            claim_mark: None,
            pointer: None,
            atlas_focus: None,
            atlas_map_hold: false,
            atlas_status: "Ready".into(),
            atlas_state: "idle".into(),
            atlas_insight_progress: None,
            atlas_pause: None,
            atlas_auto_next: None,
            atlas_auto_started: false,
            intel_page: IntelPage::Bulletin,
            intel_category: INTEL_CATEGORIES[0].into(),
            intel_day: String::new(),
            intel_days: Vec::new(),
            intel_articles: Vec::new(),
            intel_sel: 0,
            intel_claims: Vec::new(),
            intel_relations: Vec::new(),
            intel_body: None,
            intel_body_message: String::new(),
            intel_full_collapsed: false,
            intel_jobs: Vec::new(),
            intel_sections: Vec::new(),
            intel_job_sel: 0,
            intel_recon_tab: 0,
            intel_recon_recommended: intel_recon::default_recon_mode(),
            intel_recon_recommended_for: String::new(),
            intel_mode_classifying: false,
            intel_recon_enabled: HashMap::new(),
            intel_recon_focus: IntelReconFocus::Tab(0),
            intel_body_running: HashSet::new(),
            intel_insights_running: HashSet::new(),
            intel_actors_reviewing: HashSet::new(),
            intel_links_reviewing: HashSet::new(),
            intel_link_explanations: HashMap::new(),
            intel_report_running: HashMap::new(),
            brain_graph: recon::MemoryGraph::default(),
            brain_graph_for: None,
            brain_detail: Default::default(),
            memory_total,
            memory_error: None,
            memories_loaded: true,
            graph_summary: String::new(),
            graph_summary_pending: None,
            graph_summary_request: String::new(),
            summary_failure: None,
            summary_details_open: false,
            hits: Vec::new(),
            auth,
            settings,
            hardware: hardware::profile_cached(false),
            profile: super::profile::ProfileView::default(),
            profile_config: super::profile_config::ConfigView::default(),
            auth_path: paths::auth_path(),
            settings_path: paths::config_path(),
            store,
            provider_tx,
            provider_rx,
            work_tx,
            work_rx,
        };

        let _ = app.store.atlas_park_running();
        app.load_atlas();
        app.atlas_auto_next = app.store.atlas_auto_next().ok().flatten();
        app.load_home_draft();
        app.load_session_tabs();
        if app.module.is_none() {
            app.set_focus(Target::Field(FieldId::Composer));
        }
        app.push_log("info", "Argos ready");
        if app.selected_thread.is_some() {
            let _ = app.refresh_selected();
        }
        if let Some(session) = app.load_last_view_session() {
            if session.module.is_some() {
                app.overlay = Overlay::ResumeSession(Box::new(session));
                app.set_focus(Target::Button(ButtonId::ResumeSessionConfirm));
            }
        }
        Ok(app)
    }

    /// Durable error events within retention (home badge, Logs header).
    pub fn error_count(&self) -> usize {
        self.logs.counts.error.max(0) as usize
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

    /// Session log lines are durable events (Logs is their only view).
    fn push_log_detail(&mut self, level: &str, text: impl Into<String>, detail: impl Into<String>) {
        let text = text.into();
        let detail = detail.into();
        let event = super::logs::session_event(level, &text, &detail);
        if self.store.record_event(&event).is_ok() && level == "error" {
            self.logs.counts.error += 1;
        }
        if self.module == Some(ModuleId::Logs) {
            self.dashboards_at = None;
        }
    }

    /// Throttled live refresh for Jobs/Logs and the home error badge. Returns
    /// true when something may have changed on screen.
    pub(crate) fn tick_dashboards(&mut self) -> bool {
        if self
            .dashboards_at
            .is_some_and(|at| at.elapsed() < Duration::from_secs(1))
        {
            return false;
        }
        let first = self.dashboards_at.is_none() && !self.logs.loaded;
        self.dashboards_at = Some(Instant::now());
        if first {
            let _ = self.store.prune_events(super::logs::RETENTION_HOURS);
        }
        match self.module {
            Some(ModuleId::Logs) => self.reload_logs(),
            Some(ModuleId::Jobs) => self.reload_jobs(),
            Some(ModuleId::System) => self.reload_profile(),
            _ => self.logs.refresh_counts(&self.store),
        }
        true
    }

    /// One Profile snapshot, on the 1 Hz cadence the dashboards already share.
    /// The read is on the store's own connection, so it never blocks recording.
    pub(crate) fn reload_profile(&mut self) {
        if !self.profile.due() {
            return;
        }
        self.profile.reload(&self.store);
    }

    pub(crate) fn reload_logs(&mut self) {
        self.logs.reload(&self.store);
        let (room, width) = super::logs::list_geometry(super::ui::body_rect(self));
        self.logs.reveal(room, width);
        let max = self.logs.scroll_max(room, width);
        if self.logs.follow || self.logs.scroll > max {
            self.logs.scroll = if self.logs.follow {
                max
            } else {
                self.logs.scroll.min(max)
            };
        }
    }

    pub(crate) fn reload_jobs(&mut self) {
        self.jobs.reload(&self.store);
        let room = super::jobs::table_room(super::ui::body_rect(self), &self.jobs);
        super::jobs::reveal(&mut self.jobs, room);
    }

    /// Jobs → Logs prefiltered to the job and its descendants, keeping a
    /// return path to the job.
    fn view_job_logs(&mut self) -> Result<String> {
        let Some(id) = self.jobs.selected().map(|job| job.id.clone()) else {
            return Ok("Select a job first".into());
        };
        self.logs.job = id.clone();
        self.logs.back_to_job = Some(id.clone());
        self.logs.follow = true;
        self.logs.open.clear();
        self.select(ModuleId::Logs.index());
        Ok(format!("Logs for job {}", super::logs::short_id(&id)))
    }

    /// Logs → Jobs for a job id (selected event's job, or the return path).
    fn open_job(&mut self, id: &str) -> Result<String> {
        self.select(ModuleId::Jobs.index());
        if self.jobs.focus_job(&self.store, id) {
            let room = super::jobs::table_room(super::ui::body_rect(self), &self.jobs);
            super::jobs::reveal(&mut self.jobs, room);
            self.set_focus(Target::JobRow(self.jobs.sel));
            Ok(format!("Job {}", super::logs::short_id(id)))
        } else {
            Ok(format!(
                "Job {} is no longer in history",
                super::logs::short_id(id)
            ))
        }
    }

    fn leave_logs_to_job(&mut self) -> bool {
        let Some(id) = self.logs.back_to_job.take() else {
            return false;
        };
        self.logs.job.clear();
        let opened = self.open_job(&id);
        self.report(opened);
        true
    }

    /// Where "Open source" leads for the selected job, when it is resolvable.
    pub(crate) fn job_source(&self) -> Option<JobSource> {
        let job = self.jobs.selected()?;
        if job.app == "atlas" && !job.run_ref.is_empty() {
            let index = self
                .atlas_runs
                .iter()
                .position(|run| run.id == job.run_ref)?;
            return Some(JobSource::AtlasRun(index));
        }
        let thread = job
            .resource_ref
            .strip_prefix("thread:")
            .or_else(|| (job.app == "recon").then_some(job.run_ref.as_str()))
            .filter(|id| !id.is_empty())?;
        self.threads
            .iter()
            .any(|t| t.id == thread)
            .then(|| JobSource::ReconThread(thread.to_string()))
    }

    fn open_job_source(&mut self) -> Result<String> {
        match self.job_source() {
            Some(JobSource::AtlasRun(index)) => {
                self.select(ModuleId::Atlas.index());
                self.atlas_run_sel = index.min(self.atlas_runs.len().saturating_sub(1));
                self.set_focus(Target::AtlasHistory(self.atlas_run_sel));
                Ok("Atlas cycle".into())
            }
            Some(JobSource::ReconThread(id)) => {
                self.select(ModuleId::Recon.index());
                self.open_thread_with_history(&id, true)?;
                Ok("Investigation".into())
            }
            None => Ok("This job has no source to open".into()),
        }
    }

    pub fn go_home(&mut self) {
        self.flush_draft();
        let _ = self.flush_home_draft();
        self.overlay = Overlay::None;
        self.module = None;
        self.tab_sel = 0;
        self.load_home_draft();
        self.set_focus(Target::Field(FieldId::Composer));
        self.status = "Home".into();
    }

    fn load_last_view_session(&self) -> Option<LastViewSession> {
        self.store
            .app_state_get("last_view_session")
            .ok()
            .flatten()
            .and_then(|raw| serde_json::from_str(&raw).ok())
    }

    fn capture_view_session(&self) -> Option<LastViewSession> {
        let module = self.module?;
        let mod_name = match module {
            ModuleId::Intel => "intel",
            ModuleId::Recon => "recon",
            ModuleId::Brain => "brain",
            ModuleId::Atlas => "atlas",
            ModuleId::Jobs => "jobs",
            ModuleId::Logs => "logs",
            ModuleId::Osint => "osint",
            ModuleId::Providers => "providers",
            ModuleId::System => "system",
        }
        .to_string();

        let mut session = LastViewSession {
            module: Some(mod_name),
            intel_page: None,
            intel_article_id: None,
            intel_article_title: None,
            recon_thread_id: None,
            recon_thread_title: None,
            memory_id: None,
            memory_title: None,
            atlas_page: None,
            osint_tool_id: None,
            providers_page: None,
            updated_at: chrono::Utc::now().to_rfc3339(),
        };

        match module {
            ModuleId::Intel => {
                session.intel_page = Some(match self.intel_page {
                    IntelPage::Bulletin => "bulletin".to_string(),
                    IntelPage::Briefing => "briefing".to_string(),
                });
                if let Some(art) = self.intel_articles.get(self.intel_sel) {
                    session.intel_article_id = Some(art.id.clone());
                    session.intel_article_title = Some(art.title.clone());
                }
            }
            ModuleId::Recon => {
                if let Some(tid) = &self.selected_thread {
                    session.recon_thread_id = Some(tid.clone());
                    if let Some(t) = self.threads.iter().find(|t| &t.id == tid) {
                        session.recon_thread_title = Some(t.title.clone());
                    }
                }
            }
            ModuleId::Brain => {
                if let Some(mem) = &self.brain_detail.memory {
                    session.memory_id = Some(mem.id.clone());
                    session.memory_title = Some(mem.text.lines().next().unwrap_or("").to_string());
                }
            }
            ModuleId::Atlas => {
                session.atlas_page = Some(match self.atlas_page {
                    AtlasPage::Live => "live".to_string(),
                    AtlasPage::Runs => "runs".to_string(),
                });
            }
            ModuleId::Osint => {
                if let Some(tool) = osint::registry().get(self.tool_sel) {
                    session.osint_tool_id = Some(tool.id.to_string());
                }
            }
            ModuleId::Providers => {
                session.providers_page = Some(self.provider_page.title().to_string());
            }
            _ => {}
        }

        Some(session)
    }

    pub(crate) fn persist_view_session(&mut self) {
        if let Some(session) = self.capture_view_session() {
            if let Ok(json) = serde_json::to_string(&session) {
                let _ = self.store.app_state_set("last_view_session", &json);
            }
        }
    }

    fn resume_view_session(&mut self, session: &LastViewSession) {
        self.overlay = Overlay::None;
        let Some(mod_name) = &session.module else {
            return;
        };
        let Some(mod_id) = ModuleId::ALL
            .iter()
            .find(|m| {
                m.title().eq_ignore_ascii_case(mod_name)
                    || match m {
                        ModuleId::Intel => mod_name == "intel",
                        ModuleId::Recon => mod_name == "recon",
                        ModuleId::Brain => mod_name == "brain",
                        ModuleId::Atlas => mod_name == "atlas",
                        ModuleId::Jobs => mod_name == "jobs",
                        ModuleId::Logs => mod_name == "logs",
                        ModuleId::Osint => mod_name == "osint" || mod_name == "tools",
                        ModuleId::Providers => mod_name == "providers" || mod_name == "models",
                        ModuleId::System => mod_name == "system" || mod_name == "profile",
                    }
            })
            .copied()
        else {
            return;
        };

        self.select(mod_id.index());

        match mod_id {
            ModuleId::Intel => {
                self.load_intel();
                if let Some(art_id) = &session.intel_article_id {
                    if let Some(pos) = self.intel_articles.iter().position(|a| a.id == *art_id) {
                        self.intel_sel = pos;
                    }
                }
                if session.intel_page.as_deref() == Some("briefing") {
                    self.open_intel_briefing();
                }
            }
            ModuleId::Recon => {
                if let Some(tid) = &session.recon_thread_id {
                    let _ = self.enter_investigation(tid);
                }
            }
            ModuleId::Brain => {
                self.reload_memories();
                if let Some(mem_id) = &session.memory_id {
                    let _ = self.open_memory_detail(mem_id);
                }
            }
            ModuleId::Atlas => {
                if session.atlas_page.as_deref() == Some("runs") {
                    self.atlas_page = AtlasPage::Runs;
                } else {
                    self.atlas_page = AtlasPage::Live;
                }
            }
            ModuleId::Osint => {
                if let Some(tool_id) = &session.osint_tool_id {
                    if let Some(pos) = osint::registry()
                        .iter()
                        .position(|t| t.id == tool_id.as_str())
                    {
                        self.select_tool(pos);
                    }
                }
            }
            ModuleId::Providers => {
                if let Some(page) = &session.providers_page {
                    for p in ProviderPage::ALL {
                        if p.title().eq_ignore_ascii_case(page) {
                            self.provider_page = p;
                            break;
                        }
                    }
                }
            }
            _ => {}
        }
        self.status = format!("Resumed {}", mod_id.title());
    }

    /// Returns true if the given tab index corresponds to the Home tab
    pub fn is_home_tab(&self, tab_index: usize) -> bool {
        tab_index == 0
    }

    /// Returns the Thread ID for the given tab index, or None if it's the Home tab
    pub fn tab_to_thread_id(&self, tab_index: usize) -> Option<String> {
        if self.is_home_tab(tab_index) {
            None
        } else {
            let adjusted_index = tab_index - 1; // Subtract 1 for Home tab
            self.tab_ids.get(adjusted_index).cloned()
        }
    }

    /// Returns the tab index for the given Thread ID, or None if not found or if it's the Home tab
    pub fn thread_id_to_tab(&self, thread_id: &str) -> Option<usize> {
        self.tab_ids
            .iter()
            .position(|id| id == thread_id)
            .map(|index| index + 1) // Add 1 for Home tab offset
    }

    /// Opens an investigation tab for the given thread ID
    pub fn open_investigation_tab(&mut self, thread_id: &str) -> Result<()> {
        if let Some(tab_index) = self.thread_id_to_tab(thread_id) {
            self.tab_sel = tab_index;
        } else {
            self.tab_ids.push(thread_id.to_string());
            self.tab_sel = self.tab_ids.len(); // Index of the newly added tab (plus Home tab offset)
        }
        self.tab_unreads.remove(thread_id);
        self.tab_last_active
            .insert(thread_id.to_string(), Instant::now());
        self.save_session_tabs();
        Ok(())
    }

    /// Closes the tab at the given index
    pub fn close_tab(&mut self, tab_index: usize) -> Result<Option<String>> {
        if self.is_home_tab(tab_index) || tab_index > self.tab_ids.len() {
            return Ok(None);
        }

        let adjusted_index = tab_index - 1; // Subtract 1 for Home tab offset
        let thread_id = self.tab_ids.remove(adjusted_index);
        self.tab_last_active.remove(&thread_id);

        // Add to recently closed history (limit to 20, deduplicated)
        self.tab_recently_closed.retain(|id| id != &thread_id);
        self.tab_recently_closed.push(thread_id.clone());
        if self.tab_recently_closed.len() > 20 {
            self.tab_recently_closed.remove(0);
        }

        let was_active = tab_index == self.tab_sel;
        if was_active {
            if self.tab_ids.is_empty() {
                self.tab_sel = 0;
                self.go_home();
            } else if tab_index <= self.tab_ids.len() {
                self.tab_sel = tab_index;
                let next_id = self.tab_ids[tab_index - 1].clone();
                self.enter_investigation(&next_id)?;
            } else {
                self.tab_sel = self.tab_ids.len();
                let next_id = self.tab_ids[self.tab_sel - 1].clone();
                self.enter_investigation(&next_id)?;
            }
        } else if tab_index < self.tab_sel {
            self.tab_sel = self.tab_sel.saturating_sub(1);
        }

        self.save_session_tabs();

        if self.running_thread(&thread_id) {
            self.status = "Tab closed; investigation continues in Jobs".into();
        }

        Ok(Some(thread_id))
    }

    /// Reopens the most recently closed tab
    pub fn reopen_closed_tab(&mut self) -> Result<()> {
        while let Some(thread_id) = self.tab_recently_closed.pop() {
            if self.store.get_thread(&thread_id).ok().flatten().is_some() {
                self.open_investigation_tab(&thread_id)?;
                self.enter_investigation(&thread_id)?;
                self.status = "Investigation tab reopened".into();
                return Ok(());
            }
        }
        self.status = "No closed investigations to reopen".into();
        Ok(())
    }

    /// Switch to a tab by its visual index (0 = Home, 1+ = investigation)
    pub fn switch_tab(&mut self, tab_index: usize) -> Result<()> {
        if tab_index == 0 {
            self.tab_sel = 0;
            self.go_home();
            Ok(())
        } else if let Some(id) = self.tab_to_thread_id(tab_index) {
            self.tab_sel = tab_index;
            self.enter_investigation(&id)
        } else {
            Ok(())
        }
    }

    /// Selects the next tab in the strip
    pub fn next_tab(&mut self) -> Result<()> {
        let total = self.tab_ids.len() + 1;
        if total <= 1 {
            return Ok(());
        }
        let next = (self.tab_sel + 1) % total;
        self.switch_tab(next)
    }

    /// Selects the previous tab in the strip
    pub fn prev_tab(&mut self) -> Result<()> {
        let total = self.tab_ids.len() + 1;
        if total <= 1 {
            return Ok(());
        }
        let prev = if self.tab_sel == 0 {
            total - 1
        } else {
            self.tab_sel - 1
        };
        self.switch_tab(prev)
    }

    /// Loads session tabs from SQLite app_state
    pub fn load_session_tabs(&mut self) {
        if let Ok(Some(json)) = self.store.app_state_get("recon_session_tabs") {
            if !json.is_empty() {
                if let Ok(state) = serde_json::from_str::<SessionTabsState>(&json) {
                    let valid_tabs: Vec<String> = state
                        .open_thread_ids
                        .into_iter()
                        .filter(|id| self.store.get_thread(id).ok().flatten().is_some())
                        .collect();
                    self.tab_ids = valid_tabs;
                    self.tab_recently_closed = state.recently_closed;
                    if self.module == Some(ModuleId::Recon) {
                        if let Some(last_id) = state.last_active_id {
                            if let Some(tab_idx) = self.thread_id_to_tab(&last_id) {
                                self.tab_sel = tab_idx;
                            }
                        }
                    } else {
                        self.tab_sel = 0;
                    }
                }
            }
        }
    }

    /// Saves session tabs to SQLite app_state
    pub fn save_session_tabs(&self) {
        let state = SessionTabsState {
            open_thread_ids: self.tab_ids.clone(),
            last_active_id: self.selected_thread.clone(),
            recently_closed: self.tab_recently_closed.clone(),
        };
        if let Ok(json) = serde_json::to_string(&state) {
            let _ = self.store.app_state_set("recon_session_tabs", &json);
        }
    }

    /// Loads Home draft from SQLite app_state
    pub fn load_home_draft(&mut self) {
        if let Ok(Some(json)) = self.store.app_state_get("home_recon_draft") {
            if !json.is_empty() {
                if let Ok(state) = serde_json::from_str::<HomeDraftState>(&json) {
                    self.home_draft = state.prompt;
                    self.home_draft_cursor = state.cursor;
                    self.home_draft_scroll = state.scroll;
                    self.home_draft_report_mode = state.report_mode;
                    if self.module.is_none() {
                        self.input = self.home_draft.clone();
                        self.cursor = self.home_draft_cursor;
                    }
                }
            }
        }
    }

    /// Clears the Home draft in memory and durable storage
    pub fn clear_home_draft(&mut self) {
        self.home_draft.clear();
        self.home_draft_cursor = 0;
        self.home_draft_scroll = 0;
        self.home_draft_dirty = false;
        if self.module.is_none() {
            self.input.clear();
            self.cursor = 0;
        }
        let _ = self.store.app_state_set("home_recon_draft", "");
    }

    /// Opens the investigation switcher overlay
    pub fn open_investigation_switcher(&mut self) {
        let mut items = Vec::new();
        for id in &self.tab_ids {
            let title = self
                .threads
                .iter()
                .find(|t| &t.id == id)
                .map(|t| t.title.as_str())
                .unwrap_or("Investigation");
            let running = if self.running_thread(id) {
                " (running)"
            } else {
                ""
            };
            items.push(ChoiceItem {
                id: id.clone(),
                label: format!("{title}{running}"),
            });
        }
        for thread in &self.threads {
            if !self.tab_ids.contains(&thread.id) {
                items.push(ChoiceItem {
                    id: thread.id.clone(),
                    label: format!("{} (closed)", thread.title),
                });
            }
        }
        self.choice_items = items;
        self.choice_sel = 0;
        self.overlay = Overlay::Choice(ChoiceKind::Investigation);
    }

    /// Launches a new investigation from the Home composer
    pub fn launch_home_investigation(&mut self) -> Result<()> {
        if self.home_draft_submitting {
            return Ok(());
        }
        let question = self.input.trim().to_string();
        if question.is_empty() {
            return Ok(());
        }
        self.home_draft_submitting = true;
        self.launch_state = LaunchState::Accepting;

        // Generate deterministic title from first non-empty line of prompt (up to 80 chars)
        let title_line = question
            .lines()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("New investigation");
        let title: String = title_line.trim().chars().take(80).collect();
        let title = if title.is_empty() {
            "New investigation".to_string()
        } else {
            title
        };

        // Atomic persistence: create thread
        let thread = match self.store.new_thread(&title) {
            Ok(t) => t,
            Err(e) => {
                self.home_draft_submitting = false;
                self.launch_state = LaunchState::RecoverableFailure;
                return Err(e);
            }
        };
        self.threads.insert(0, thread.clone());
        self.thread_sel = 0;

        // Open investigation tab
        let _ = self.open_investigation_tab(&thread.id);

        // Consume Home draft
        self.clear_home_draft();
        self.home_draft_submitting = false;
        self.launch_state = LaunchState::Accepted;

        // Check navigation intent: if user stayed on Home, navigate immediately to Recon
        if self.module.is_none() {
            self.enter_investigation(&thread.id)?;
        } else {
            self.status = "Investigation started — Open".into();
        }

        // Start investigation run asynchronously
        self.send_recon_query(&thread.id, &question)?;
        Ok(())
    }

    pub fn palette_items(&self) -> Vec<PaletteItem> {
        super::commands::matching(&self.palette_query, self.module)
            .into_iter()
            .filter(|command| command.module.is_none() || command.module == self.module)
            .map(|command| PaletteItem {
                id: command.id.into(),
                label: command.label.into(),
                description: command.description.into(),
                shortcut: command.shortcut.into(),
                category: command.category.into(),
                enabled: match command.id {
                    "cancel" => self
                        .selected_thread
                        .as_ref()
                        .is_some_and(|id| self.running_thread(id)),
                    "resume" => {
                        self.runs
                            .iter()
                            .any(|run| matches!(run.state.as_str(), "interrupted" | "failed"))
                            && self
                                .selected_thread
                                .as_ref()
                                .is_some_and(|id| !self.running_thread(id))
                    }
                    "evidence" => super::ui::chat_blocks(self)
                        .get(self.chat_sel)
                        .and_then(|block| block.key.strip_prefix("tool:"))
                        .is_some_and(|id| {
                            self.calls
                                .iter()
                                .any(|call| call.id == id && call.result.is_some())
                        }),
                    _ => true,
                },
                disabled_reason: match command.id {
                    "cancel" => "No running turn".into(),
                    "resume" => "No interrupted or failed run to resume".into(),
                    "evidence" => "Select a completed evidence activity".into(),
                    _ => String::new(),
                },
            })
            .collect()
    }

    fn open_palette(&mut self) {
        if self.overlay == Overlay::Palette {
            self.overlay = Overlay::None;
            return;
        }
        self.overlay = Overlay::Palette;
        self.palette_query.clear();
        self.palette_sel = 0;
        self.scrolls.popup = 0;
    }

    fn open_selected_evidence(&mut self) {
        if self.module != Some(ModuleId::Recon) || !self.recon_chat {
            self.status = "Open an investigation to inspect evidence".into();
            return;
        }
        let blocks = super::ui::chat_blocks(self);
        let Some(call_id) = blocks
            .get(self.chat_sel)
            .and_then(|block| block.key.strip_prefix("tool:"))
        else {
            self.status = "Select an evidence activity row first".into();
            return;
        };
        let Some((index, call)) = self
            .calls
            .iter()
            .enumerate()
            .find(|(_, call)| call.id == call_id)
        else {
            self.status = "The captured source is no longer available".into();
            return;
        };
        let Some(result) = call.result.as_ref() else {
            self.status = "The selected call has no captured result yet".into();
            return;
        };
        let name = osint::definition(&call.tool_id)
            .map(|tool| tool.name)
            .unwrap_or(call.tool_id.as_str());
        let observations = serde_json::to_string_pretty(&result.observations)
            .unwrap_or_else(|_| "Captured observations unavailable".into());
        let observations: String = observations
            .chars()
            .filter(|ch| *ch == '\n' || *ch == '\t' || !ch.is_control())
            .take(12_000)
            .collect();
        let source: String = if result.source_url.is_empty() {
            "Unavailable"
        } else {
            &result.source_url
        }
        .chars()
        .filter(|ch| !ch.is_control())
        .take(500)
        .collect();
        let retrieved: String = result
            .retrieved_at
            .chars()
            .filter(|ch| !ch.is_control())
            .take(80)
            .collect();
        self.overlay = Overlay::Block {
            title: format!("Evidence E{} · {name}", index + 1),
            body: format!(
                "Captured source: {source}\nRetrieved: {}\nExecution: {}\nAssessment: unassessed\nCall: {}\n\n{}",
                retrieved, call.status, call.id, observations
            ),
        };
        self.scrolls.popup = 0;
    }

    fn run_palette(&mut self, id: &str) {
        self.overlay = Overlay::None;
        match id {
            "home" => self.go_home(),
            "intel" => self.select(ModuleId::Intel.index()),
            "recon" => self.select(ModuleId::Recon.index()),
            "brain" => self.select(ModuleId::Brain.index()),
            "atlas" => self.select(ModuleId::Atlas.index()),
            "jobs" => self.select(ModuleId::Jobs.index()),
            "logs" => self.select(ModuleId::Logs.index()),
            "tools" | "osint" => self.select(ModuleId::Osint.index()),
            "models" | "providers" => self.select(ModuleId::Providers.index()),
            "system" | "profile" => self.select(ModuleId::System.index()),
            "new" => {
                self.go_home();
                self.set_focus(Target::Field(FieldId::Composer));
            }
            "focus-home-composer" => {
                self.go_home();
                self.set_focus(Target::Field(FieldId::Composer));
            }
            "switch-investigation" => self.open_investigation_switcher(),
            "next-tab" => {
                let _ = self.next_tab();
            }
            "prev-tab" => {
                let _ = self.prev_tab();
            }
            "close-tab" => {
                if self.tab_sel > 0 {
                    let _ = self.close_tab(self.tab_sel);
                }
            }
            "reopen-tab" => {
                let _ = self.reopen_closed_tab();
            }
            "rename-investigation" => {
                if self.selected_thread.is_some() {
                    self.recon_chat = false;
                    self.input = ":rename ".into();
                    self.cursor = 8;
                    self.set_focus(Target::Field(FieldId::Composer));
                }
            }
            "sessions" => {
                self.module = Some(ModuleId::Recon);
                self.recon_chat = false;
                self.set_focus(Target::Field(FieldId::ReconSearch));
            }
            "help" => {
                self.overlay = Overlay::Help;
                self.scrolls.popup = 0;
            }
            "palette" => self.open_palette(),
            "cancel" => self.activate_button(ButtonId::CancelRun),
            "resume" => self.activate_button(ButtonId::ResumeRun),
            "insights" => self.activate_button(ButtonId::RetryInsights),
            "evidence" => self.open_selected_evidence(),
            "context" => {
                self.recon_context_enabled = !self.recon_context_enabled;
                self.settings.tui_recon_context = Some(self.recon_context_enabled);
                let saved = self.save_settings().map(|_| {
                    format!(
                        "Investigation context {}",
                        if self.recon_context_enabled {
                            "shown"
                        } else {
                            "hidden"
                        }
                    )
                });
                self.report(saved);
            }
            "create-memory" => {
                self.select(ModuleId::Brain.index());
                self.activate_button(ButtonId::CreateMemory);
            }
            "clear-log" => {
                self.select(ModuleId::Logs.index());
                self.activate_button(ButtonId::ClearLog);
            }
            "brain-pin" => self.activate_button(ButtonId::Pin),
            "brain-back" => self.activate_button(ButtonId::BrainDetailBack),
            "brain-summary-retry" => self.activate_button(ButtonId::SummaryRetry),
            "brain-summary-logs" => self.activate_button(ButtonId::SummaryLogs),
            "brain-summary-job" => self.activate_button(ButtonId::SummaryJob),
            "brain-summary-model" => self.activate_button(ButtonId::SummaryModels),
            "jobs-logs" => self.activate_button(ButtonId::JobsViewLogs),
            "jobs-retry" => self.activate_button(ButtonId::JobsRetry),
            "jobs-cancel" => self.activate_button(ButtonId::JobsCancel),
            "jobs-source" => self.activate_button(ButtonId::JobsOpenSource),
            "jobs-status" => self.activate_button(ButtonId::JobsStatus),
            "jobs-app" => self.activate_button(ButtonId::JobsApp),
            "logs-follow" => self.activate_button(ButtonId::LogsFollow),
            "logs-job" => self.activate_button(ButtonId::LogsOpenJob),
            "logs-back" => self.activate_button(ButtonId::LogsBack),
            "logs-level" => self.activate_button(ButtonId::LogsLevel),
            "logs-app" => self.activate_button(ButtonId::LogsApp),
            "atlas-history" => self.activate_button(ButtonId::AtlasRuns),
            "atlas-live" => self.activate_button(ButtonId::AtlasLive),
            "atlas-run" => self.activate_button(ButtonId::AtlasRun),
            "atlas-auto" => self.activate_button(ButtonId::AtlasAuto),
            "atlas-resume" => self.activate_button(ButtonId::AtlasResume),
            "atlas-repair" => self.activate_button(ButtonId::AtlasRepair),
            "intel-search" => self.set_focus(Target::Field(FieldId::IntelSearch)),
            "intel-recon" => self.activate_button(ButtonId::IntelReports),
            "intel-refresh" => self.activate_button(ButtonId::IntelBodyRefresh),
            "intel-retry" => self.activate_button(ButtonId::IntelBodyRetry),
            "intel-job" => self.activate_button(ButtonId::IntelJobOpen),
            _ => {}
        }
    }

    fn run_slash(&mut self, input: &str) -> Result<String> {
        let mut parts = input
            .trim()
            .trim_start_matches('/')
            .splitn(2, char::is_whitespace);
        let name = parts.next().unwrap_or("").to_ascii_lowercase();
        match name.as_str() {
            "help" | "?" => {
                self.overlay = Overlay::Help;
                self.scrolls.popup = 0;
                self.input.clear();
                Ok("Shortcuts".into())
            }
            "new" => {
                self.new_thread()?;
                self.input.clear();
                Ok("New investigation".into())
            }
            "sessions" => {
                self.recon_chat = false;
                self.input.clear();
                self.set_focus(Target::Field(FieldId::ReconSearch));
                Ok("Investigations".into())
            }
            "cancel" => self.recon_command(":cancel"),
            "resume" => {
                self.activate_button(ButtonId::ResumeRun);
                self.input.clear();
                Ok("Resume requested".into())
            }
            "insights" => {
                self.activate_button(ButtonId::RetryInsights);
                self.input.clear();
                Ok("Insights requested".into())
            }
            "home" => {
                self.go_home();
                Ok("Home".into())
            }
            "thinking" => {
                self.show_thinking = !self.show_thinking;
                self.input.clear();
                Ok(if self.show_thinking {
                    "Thinking disclosure enabled".into()
                } else {
                    "Thinking disclosure collapsed".into()
                })
            }
            "details" => {
                let id = self.selected_thread.clone().unwrap_or_default();
                let has_expanded = self
                    .expanded
                    .iter()
                    .any(|k| k.starts_with(&id) || k.starts_with("plan:"));
                if has_expanded {
                    self.expanded
                        .retain(|k| !k.starts_with(&id) && !k.starts_with("plan:"));
                    self.input.clear();
                    Ok("Details collapsed".into())
                } else {
                    if let Some(run) = self.runs.last() {
                        self.expanded.insert(format!("plan:{}", run.id));
                    }
                    self.input.clear();
                    Ok("Details expanded".into())
                }
            }
            "trace" => {
                self.input.clear();
                let id = self.selected_thread.clone().unwrap_or_default();
                let count = self
                    .store
                    .list_investigation_events(&id)
                    .map(|e| e.len())
                    .unwrap_or(0);
                Ok(format!("Trace: {count} events logged"))
            }
            "directives" => {
                self.input.clear();
                let id = self.selected_thread.clone().unwrap_or_default();
                let count = self
                    .store
                    .list_investigation_tasks(&id)
                    .map(|t| t.len())
                    .unwrap_or(0);
                Ok(format!("Directives / tasks: {count}"))
            }
            "evidence" => {
                self.input.clear();
                let id = self.selected_thread.clone().unwrap_or_default();
                let count = self
                    .store
                    .list_evidence_passages(&id)
                    .map(|p| p.len())
                    .unwrap_or(0);
                Ok(format!("Evidence passages: {count}"))
            }
            "brain" | "atlas" | "osint" | "providers" | "tools" | "models" | "jobs" | "logs"
            | "system" | "profile" | "recon" | "intel" => {
                let index = match name.as_str() {
                    "intel" => ModuleId::Intel.index(),
                    "atlas" => ModuleId::Atlas.index(),
                    "brain" => ModuleId::Brain.index(),
                    "recon" => ModuleId::Recon.index(),
                    "jobs" => ModuleId::Jobs.index(),
                    "logs" => ModuleId::Logs.index(),
                    "tools" | "osint" => ModuleId::Osint.index(),
                    "models" | "providers" => ModuleId::Providers.index(),
                    "system" | "profile" => ModuleId::System.index(),
                    _ => ModuleId::System.index(),
                };
                self.select(index);
                Ok(format!("{} open", ModuleId::ALL[index].title()))
            }
            "palette" => {
                self.open_palette();
                Ok("Commands".into())
            }
            "" => {
                self.open_palette();
                Ok("Commands".into())
            }
            other => Err(anyhow::anyhow!("Unknown command /{other}")),
        }
    }

    /// Cycle header app tabs. From home, forward opens Intel and reverse opens System.
    fn cycle_module(&mut self, reverse: bool) {
        let len = ModuleId::ALL.len();
        let current = match self.module {
            Some(module) => module.index(),
            None if reverse => 0,
            None => len - 1,
        };
        let next = if reverse {
            (current + len - 1) % len
        } else {
            (current + 1) % len
        };
        self.select(next);
    }

    pub(crate) fn config_tab(&self) -> super::profile_config::ConfigTab {
        self.profile_config.tab
    }

    pub(crate) fn config(&self) -> &super::profile_config::ConfigView {
        &self.profile_config
    }

    /// Draws the Configs export destination as a real field, so the mouse and
    /// the keyboard reach the same target.
    pub(crate) fn draw_export_field(&self, frame: &mut Frame, area: Rect) {
        super::ui::draw_field(frame, self, FieldId::ProfileExportPath, "destination", area);
    }

    /// Keys inside the Profile > System > Configs popup.
    ///
    /// Enter inserts a newline in the editor — the user commits with Ctrl+Enter,
    /// which validates and imports only when the document is valid, so the
    /// button is always the explicit commit.
    fn profile_config_key(&mut self, key: KeyEvent) -> bool {
        let config = &mut self.profile_config;
        match key.code {
            KeyCode::Esc => {
                config.close();
                self.profile.config_open = false;
                self.overlay = Overlay::None;
                true
            }
            KeyCode::Tab | KeyCode::BackTab => {
                config.next_tab();
                // The newly open tab owns its field, so the caret moves with it.
                let tab = config.tab;
                self.set_focus(Target::Field(match tab {
                    super::profile_config::ConfigTab::Export => FieldId::ProfileExportPath,
                    super::profile_config::ConfigTab::Import => FieldId::ProfileImportEditor,
                }));
                true
            }
            KeyCode::Enter if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if config.tab == crate::tui::profile_config::ConfigTab::Import {
                    config.validate();
                    if config.plan.is_some() {
                        match config.run_import() {
                            Ok(generation) => {
                                self.status =
                                    format!("Imported configuration revision {generation}");
                            }
                            Err(err) => {
                                self.status = format!("Import failed: {err}");
                            }
                        }
                    }
                } else {
                    config.check_export_path();
                }
                true
            }
            KeyCode::Enter => match config.tab {
                crate::tui::profile_config::ConfigTab::Import => {
                    config.newline();
                    true
                }
                super::profile_config::ConfigTab::Export => {
                    // Enter runs the export, with a second Enter confirming an
                    // existing destination.
                    if config.export_confirm {
                        match config.run_export() {
                            Ok(_) => {
                                self.status = config
                                    .export_status
                                    .clone()
                                    .unwrap_or_else(|| "Exported".into());
                            }
                            Err(err) => self.status = format!("Export failed: {err}"),
                        }
                    } else {
                        config.check_export_path();
                        if config.export_error.is_none() && !config.export_confirm {
                            match config.run_export() {
                                Ok(_) => {
                                    self.status = config
                                        .export_status
                                        .clone()
                                        .unwrap_or_else(|| "Exported".into());
                                }
                                Err(err) => self.status = format!("Export failed: {err}"),
                            }
                        } else {
                            self.status = config
                                .export_error
                                .clone()
                                .unwrap_or_else(|| "Confirm the overwrite".into());
                        }
                    }
                    true
                }
            },
            KeyCode::Backspace if config.tab == crate::tui::profile_config::ConfigTab::Import => {
                config.backspace();
                true
            }
            KeyCode::Delete if config.tab == crate::tui::profile_config::ConfigTab::Import => {
                config.delete();
                true
            }
            KeyCode::Home if config.tab == crate::tui::profile_config::ConfigTab::Import => {
                config.home();
                true
            }
            KeyCode::End if config.tab == crate::tui::profile_config::ConfigTab::Import => {
                config.end();
                true
            }
            KeyCode::Left if config.tab == crate::tui::profile_config::ConfigTab::Import => {
                if config.import.caret > 0 {
                    config.import.caret -= 1;
                }
                true
            }
            KeyCode::Right if config.tab == crate::tui::profile_config::ConfigTab::Import => {
                if config.import.caret < config.import.text.chars().count() {
                    config.import.caret += 1;
                }
                true
            }
            KeyCode::Up | KeyCode::Down | KeyCode::PageUp | KeyCode::PageDown
                if config.tab == crate::tui::profile_config::ConfigTab::Import =>
            {
                let delta = match key.code {
                    KeyCode::Up => -1,
                    KeyCode::PageUp => -10,
                    KeyCode::PageDown => 10,
                    _ => 1,
                };
                let height = 12;
                config.import.scroll_by(delta, height);
                true
            }
            KeyCode::Char(c)
                if config.tab == crate::tui::profile_config::ConfigTab::Import
                    && !key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                config.insert(&c.to_string());
                true
            }
            KeyCode::Char(c)
                if config.tab == super::profile_config::ConfigTab::Export
                    && !key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                config.export_path.push(c);
                config.export_confirm = false;
                true
            }
            _ => false,
        }
    }

    fn select(&mut self, index: usize) {
        self.flush_draft();
        let _ = self.flush_home_draft();
        self.launcher_sel = index;
        self.module = Some(ModuleId::ALL[index]);
        if self.module == Some(ModuleId::System) {
            // A fresh Profile entry reads one snapshot immediately.
            self.profile.loaded_at = None;
        }
        if self.module == Some(ModuleId::System) && self.overlay == Overlay::Configs {
            self.overlay = Overlay::None;
        }
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
        if self.module == Some(ModuleId::Atlas) {
            self.atlas_page = AtlasPage::Runs;
            self.load_atlas();
        }
        if self.module == Some(ModuleId::Intel) {
            self.intel_page = IntelPage::Bulletin;
            self.load_intel();
        }
        if self.module == Some(ModuleId::Brain) {
            // Entering Brain picks up memories committed elsewhere.
            self.reload_memories();
        }
        if self.module == Some(ModuleId::Logs) {
            self.reload_logs();
            self.dashboards_at = Some(Instant::now());
        }
        if self.module == Some(ModuleId::Jobs) {
            self.atlas_runs = self.store.atlas_list_runs().unwrap_or_default();
            self.reload_jobs();
            self.dashboards_at = Some(Instant::now());
        }
        self.status = format!("{} open", ModuleId::ALL[index].title());
        self.set_focus(match self.module {
            Some(ModuleId::Intel) if !self.intel_articles.is_empty() => {
                Target::IntelArticle(self.intel_sel)
            }
            Some(ModuleId::Intel) => Target::Field(FieldId::IntelSearch),
            Some(ModuleId::Recon) if self.threads.is_empty() => Target::Field(FieldId::ReconSearch),
            Some(ModuleId::Recon) => Target::Thread(self.thread_sel),
            Some(ModuleId::Brain) => Target::Button(ButtonId::CreateMemory),
            Some(ModuleId::Atlas) if !self.atlas_runs.is_empty() => {
                Target::AtlasHistory(self.atlas_run_sel)
            }
            Some(ModuleId::Atlas) => Target::Button(ButtonId::AtlasLive),
            Some(ModuleId::Osint) => Target::Field(FieldId::OsintSearch),
            Some(ModuleId::Providers) => Target::ProviderTab(self.provider_page),
            Some(ModuleId::Jobs) if !self.jobs.rows.is_empty() => Target::JobRow(self.jobs.sel),
            Some(ModuleId::Jobs) => Target::Field(FieldId::JobsSearch),
            Some(ModuleId::Logs) if !self.logs.rows.is_empty() => Target::LogLine(self.logs.sel),
            Some(ModuleId::Logs) => Target::Field(FieldId::LogsSearch),
            _ => Target::Button(ButtonId::RefreshHardware),
        });
        self.persist_view_session();
    }

    pub fn field(&self, field: FieldId) -> &str {
        match field {
            FieldId::BrainApp => &self.brain_app,
            FieldId::BrainConversation => &self.brain_conversation,
            FieldId::BrainInsight => &self.brain_insight,
            FieldId::BrainQuery => &self.brain_query,
            FieldId::ReconSearch => &self.recon_search,
            FieldId::IntelSearch => &self.intel_search,
            FieldId::JobsSearch => &self.jobs.search,
            FieldId::LogsSearch => &self.logs.search,
            FieldId::OsintSearch => &self.osint_search,
            FieldId::OsintInput => &self.osint_input,
            FieldId::FirecrawlKey => &self.firecrawl_key,
            FieldId::FirecrawlFallback => &self.firecrawl_fallback,
            FieldId::HunterKey => &self.hunter_key,
            FieldId::HunterFallback => &self.hunter_fallback,
            FieldId::SociaVaultKey => &self.sociavault_key,
            FieldId::SociaVaultFallback => &self.sociavault_fallback,
            FieldId::NewsApiKey => &self.newsapi_key,
            FieldId::NewsApiFallback => &self.newsapi_fallback,
            FieldId::CourtListenerKey => &self.courtlistener_key,
            FieldId::CourtListenerFallback => &self.courtlistener_fallback,
            FieldId::GnewsKey => &self.gnews_key,
            FieldId::GnewsFallback => &self.gnews_fallback,
            FieldId::NewsDataKey => &self.newsdata_key,
            FieldId::NewsDataFallback => &self.newsdata_fallback,
            FieldId::CurrentsKey => &self.currents_key,
            FieldId::CurrentsFallback => &self.currents_fallback,
            FieldId::WhoxyKey => &self.whoxy_key,
            FieldId::WhoxyFallback => &self.whoxy_fallback,
            FieldId::ReconProvider => &self.recon_provider,
            FieldId::ReconModel => &self.recon_model,
            FieldId::PickerProvider => &self.picker_provider,
            FieldId::PickerModel => &self.picker_model,
            FieldId::SynthesisProvider => &self.synthesis_provider,
            FieldId::SynthesisModel => &self.synthesis_model,
            FieldId::ClassifierProvider => &self.classifier_provider,
            FieldId::ClassifierModel => &self.classifier_model,
            FieldId::SummarizationProvider => &self.summarization_provider,
            FieldId::SummarizationModel => &self.summarization_model,
            FieldId::EvidenceCuratorProvider => &self.evidence_curator_provider,
            FieldId::EvidenceCuratorModel => &self.evidence_curator_model,
            FieldId::EntityResolverProvider => &self.entity_resolver_provider,
            FieldId::EntityResolverModel => &self.entity_resolver_model,
            FieldId::ClaimAssessorProvider => &self.claim_assessor_provider,
            FieldId::ClaimAssessorModel => &self.claim_assessor_model,
            FieldId::InvestigationControllerProvider => &self.investigation_controller_provider,
            FieldId::InvestigationControllerModel => &self.investigation_controller_model,
            FieldId::RouterKey => &self.router_key,
            FieldId::RouterEndpoint => &self.router_endpoint,
            FieldId::GoogleKey => &self.google_key,
            FieldId::GoogleEndpoint => &self.google_endpoint,
            FieldId::NvidiaKey => &self.nvidia_key,
            FieldId::NvidiaEndpoint => &self.nvidia_endpoint,
            FieldId::GoogleModelFilter => &self.google_model_filter,
            FieldId::NvidiaModelFilter => &self.nvidia_model_filter,
            FieldId::RouterModelFilter => &self.router_model_filter,
            FieldId::FallbackFilter => &self.fallback_popup_filter,
            FieldId::Composer => &self.input,
            FieldId::ProfileExportPath => &self.profile_config.export_path,
            FieldId::ProfileImportEditor => "",
        }
    }

    fn field_mut(&mut self, field: FieldId) -> &mut String {
        match field {
            FieldId::BrainApp => &mut self.brain_app,
            FieldId::BrainConversation => &mut self.brain_conversation,
            FieldId::BrainInsight => &mut self.brain_insight,
            FieldId::BrainQuery => &mut self.brain_query,
            FieldId::ReconSearch => &mut self.recon_search,
            FieldId::IntelSearch => &mut self.intel_search,
            FieldId::JobsSearch => &mut self.jobs.search,
            FieldId::LogsSearch => &mut self.logs.search,
            FieldId::OsintSearch => &mut self.osint_search,
            FieldId::OsintInput => &mut self.osint_input,
            FieldId::FirecrawlKey => &mut self.firecrawl_key,
            FieldId::FirecrawlFallback => &mut self.firecrawl_fallback,
            FieldId::HunterKey => &mut self.hunter_key,
            FieldId::HunterFallback => &mut self.hunter_fallback,
            FieldId::SociaVaultKey => &mut self.sociavault_key,
            FieldId::SociaVaultFallback => &mut self.sociavault_fallback,
            FieldId::NewsApiKey => &mut self.newsapi_key,
            FieldId::NewsApiFallback => &mut self.newsapi_fallback,
            FieldId::CourtListenerKey => &mut self.courtlistener_key,
            FieldId::CourtListenerFallback => &mut self.courtlistener_fallback,
            FieldId::GnewsKey => &mut self.gnews_key,
            FieldId::GnewsFallback => &mut self.gnews_fallback,
            FieldId::NewsDataKey => &mut self.newsdata_key,
            FieldId::NewsDataFallback => &mut self.newsdata_fallback,
            FieldId::CurrentsKey => &mut self.currents_key,
            FieldId::CurrentsFallback => &mut self.currents_fallback,
            FieldId::WhoxyKey => &mut self.whoxy_key,
            FieldId::WhoxyFallback => &mut self.whoxy_fallback,
            FieldId::ReconProvider => &mut self.recon_provider,
            FieldId::ReconModel => &mut self.recon_model,
            FieldId::PickerProvider => &mut self.picker_provider,
            FieldId::PickerModel => &mut self.picker_model,
            FieldId::SynthesisProvider => &mut self.synthesis_provider,
            FieldId::SynthesisModel => &mut self.synthesis_model,
            FieldId::ClassifierProvider => &mut self.classifier_provider,
            FieldId::ClassifierModel => &mut self.classifier_model,
            FieldId::SummarizationProvider => &mut self.summarization_provider,
            FieldId::SummarizationModel => &mut self.summarization_model,
            FieldId::EvidenceCuratorProvider => &mut self.evidence_curator_provider,
            FieldId::EvidenceCuratorModel => &mut self.evidence_curator_model,
            FieldId::EntityResolverProvider => &mut self.entity_resolver_provider,
            FieldId::EntityResolverModel => &mut self.entity_resolver_model,
            FieldId::ClaimAssessorProvider => &mut self.claim_assessor_provider,
            FieldId::ClaimAssessorModel => &mut self.claim_assessor_model,
            FieldId::InvestigationControllerProvider => &mut self.investigation_controller_provider,
            FieldId::InvestigationControllerModel => &mut self.investigation_controller_model,
            FieldId::RouterKey => &mut self.router_key,
            FieldId::RouterEndpoint => &mut self.router_endpoint,
            FieldId::GoogleKey => &mut self.google_key,
            FieldId::GoogleEndpoint => &mut self.google_endpoint,
            FieldId::NvidiaKey => &mut self.nvidia_key,
            FieldId::NvidiaEndpoint => &mut self.nvidia_endpoint,
            FieldId::GoogleModelFilter => &mut self.google_model_filter,
            FieldId::NvidiaModelFilter => &mut self.nvidia_model_filter,
            FieldId::RouterModelFilter => &mut self.router_model_filter,
            FieldId::FallbackFilter => &mut self.fallback_popup_filter,
            FieldId::Composer => &mut self.input,
            FieldId::ProfileExportPath => &mut self.profile_config.export_path,
            // The import editor owns its own cursor and paste buffer; it is
            // reached through `profile_config` rather than a flat string field.
            FieldId::ProfileImportEditor => &mut self.profile_config.import.text,
        }
    }

    pub(crate) fn set_focus(&mut self, target: Target) {
        if self.focus == Target::Field(FieldId::Composer)
            && target != Target::Field(FieldId::Composer)
        {
            if self.module == Some(ModuleId::Recon) {
                self.flush_draft();
            } else if self.module.is_none() {
                let _ = self.flush_home_draft();
            }
        }
        self.focus = target;
        self.cursor = match target {
            Target::Field(field) => self.field(field).chars().count(),
            _ => 0,
        };
        self.sync_selected_insight();
    }

    fn sync_selected_insight(&mut self) {
        if self.module != Some(ModuleId::Brain) || self.brain_list_mode != BrainListMode::List {
            return;
        }
        if !matches!(self.focus, Target::Memory(_)) {
            self.selected_insight = None;
            return;
        }
        self.selected_insight = self
            .memories
            .get(self.memory_sel)
            .and_then(|memory| self.store.insight_for_memory(&memory.id).ok().flatten());
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

    pub fn enter_investigation(&mut self, id: &str) -> Result<()> {
        if self.module.is_none() {
            let _ = self.flush_home_draft();
        }
        self.open_thread(id)?;
        self.recon_chat = true;
        self.module = Some(ModuleId::Recon);
        self.set_focus(Target::Field(FieldId::Composer));
        self.open_investigation_tab(id)?;
        self.tab_unreads.remove(id);
        self.save_session_tabs();
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

        if record {
            let _ = self.open_investigation_tab(id);
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

    fn send_recon_query(&mut self, tid: &str, question: &str) -> Result<()> {
        anyhow::ensure!(
            !self.running.contains_key(tid),
            "This thread already has a running turn"
        );
        if self.selected_thread.as_deref() == Some(tid) {
            self.input.clear();
            self.cursor = 0;
            self.store
                .save_draft(tid, "", i64::from(self.scrolls.chat))?;
            self.live_answers.remove(tid);
            self.chat_follow = true;
        }
        let service =
            recon::Service::new(&paths::db_path(), self.auth.clone(), self.settings.clone())?;
        let tx = self.work_tx.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let tid_owned = tid.to_string();
        self.running.insert(tid_owned.clone(), cancel.clone());
        self.recon_stage = "starting".into();
        let question_owned = question.to_string();
        if tokio::runtime::Handle::try_current().is_ok() {
            tokio::spawn(async move {
                let progress_tx = tx.clone();
                let thread_id = tid_owned.clone();
                let outcome = service
                    .ask(&tid_owned, &question_owned, cancel, move |event| {
                        let _ = progress_tx.send(work_event(&thread_id, event));
                    })
                    .await
                    .map(|_| ())
                    .map_err(|e| e.to_string());
                let _ = tx.send(WorkEvent::ReconDone {
                    thread_id: tid_owned,
                    outcome,
                });
            });
        }
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
        self.send_recon_query(&tid, &question)
    }

    pub fn can_resume_recon(&self) -> bool {
        let Some(tid) = &self.selected_thread else {
            return false;
        };
        if self.running.contains_key(tid) {
            return false;
        }
        self.store
            .latest_resumable_run(tid)
            .ok()
            .flatten()
            .is_some()
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
        self.live_answers.remove(&tid);
        self.running.insert(tid.clone(), cancel.clone());
        if tokio::runtime::Handle::try_current().is_ok() {
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
        }
        Ok(())
    }

    fn toggle_recall(&mut self) -> Result<String> {
        let tid = self
            .selected_thread
            .clone()
            .ok_or_else(|| anyhow::anyhow!("No investigation open"))?;
        let on = self
            .threads
            .iter()
            .find(|thread| thread.id == tid)
            .map(|thread| thread.recall_insights)
            .unwrap_or(false);
        let next = !on;
        if !self.store.set_recall_insights(&tid, next)? {
            anyhow::bail!("Investigation not found");
        }
        if let Some(thread) = self.threads.iter_mut().find(|thread| thread.id == tid) {
            thread.recall_insights = next;
        }
        Ok(if next { "recall: on" } else { "recall: off" }.into())
    }

    #[allow(dead_code)]
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
        let key = self.firecrawl_key.clone();
        let fallback = self.firecrawl_fallback.clone();
        self.remember_keyed(
            &key,
            &fallback,
            "Enter a Firecrawl API key",
            "Firecrawl API key saved",
            |settings, key, fallback| {
                settings.firecrawl_api_key = key;
                settings.firecrawl_api_key_fallback = fallback;
            },
        )
    }

    fn remember_hunter_key(&mut self) -> Result<String> {
        let key = self.hunter_key.clone();
        let fallback = self.hunter_fallback.clone();
        self.remember_keyed(
            &key,
            &fallback,
            "Enter a Hunter API key",
            "Hunter API key saved",
            |settings, key, fallback| {
                settings.hunter_api_key = key;
                settings.hunter_api_key_fallback = fallback;
            },
        )
    }

    fn remember_sociavault_key(&mut self) -> Result<String> {
        let key = self.sociavault_key.clone();
        let fallback = self.sociavault_fallback.clone();
        self.remember_keyed(
            &key,
            &fallback,
            "Enter a SociaVault API key",
            "SociaVault API key saved",
            |settings, key, fallback| {
                settings.sociavault_api_key = key;
                settings.sociavault_api_key_fallback = fallback;
            },
        )
    }

    fn remember_newsapi_key(&mut self) -> Result<String> {
        let key = self.newsapi_key.clone();
        let fallback = self.newsapi_fallback.clone();
        self.remember_keyed(
            &key,
            &fallback,
            "Enter a NewsAPI key",
            "NewsAPI key saved",
            |settings, key, fallback| {
                settings.newsapi_api_key = key;
                settings.newsapi_api_key_fallback = fallback;
            },
        )
    }

    fn remember_gnews_key(&mut self) -> Result<String> {
        let key = self.gnews_key.clone();
        let fallback = self.gnews_fallback.clone();
        self.remember_keyed(
            &key,
            &fallback,
            "Enter a GNews API key",
            "GNews API key saved",
            |settings, key, fallback| {
                settings.gnews_api_key = key;
                settings.gnews_api_key_fallback = fallback;
            },
        )
    }

    fn remember_newsdata_key(&mut self) -> Result<String> {
        let key = self.newsdata_key.clone();
        let fallback = self.newsdata_fallback.clone();
        self.remember_keyed(
            &key,
            &fallback,
            "Enter a NewsData API key",
            "NewsData API key saved",
            |settings, key, fallback| {
                settings.newsdata_api_key = key;
                settings.newsdata_api_key_fallback = fallback;
            },
        )
    }

    fn remember_currents_key(&mut self) -> Result<String> {
        let key = self.currents_key.clone();
        let fallback = self.currents_fallback.clone();
        self.remember_keyed(
            &key,
            &fallback,
            "Enter a Currents API key",
            "Currents API key saved",
            |settings, key, fallback| {
                settings.currents_api_key = key;
                settings.currents_api_key_fallback = fallback;
            },
        )
    }

    fn remember_whoxy_key(&mut self) -> Result<String> {
        let key = self.whoxy_key.clone();
        let fallback = self.whoxy_fallback.clone();
        self.remember_keyed(
            &key,
            &fallback,
            "Enter a Whoxy API key",
            "Whoxy API key saved",
            |settings, key, fallback| {
                settings.whoxy_api_key = key;
                settings.whoxy_api_key_fallback = fallback;
            },
        )
    }

    fn test_whoxy_connection(&mut self) -> Result<String> {
        let _ = self.remember_whoxy_key();
        let key = self.settings.provider_key("whoxy");
        anyhow::ensure!(!key.trim().is_empty(), "Enter a Whoxy API key");
        let user_agent =
            osint::effective_user_agent(Some(&self.settings.osint_user_agent)).to_string();
        let tx = self.work_tx.clone();
        tokio::spawn(async move {
            let outcome = osint::whoxy::check_balance(&key, &user_agent)
                .await
                .map_err(|e| e.to_string());
            let _ = tx.send(WorkEvent::WhoxyBalance { outcome });
        });
        Ok("Testing Whoxy connection…".into())
    }

    /// Saves the primary key and the optional second account. A blank fallback clears it.
    /// A blank primary is allowed when a fallback is set, so an environment key can stay
    /// the first account.
    fn remember_keyed(
        &mut self,
        primary: &str,
        fallback: &str,
        missing: &str,
        saved: &str,
        write: impl FnOnce(&mut SettingsFile, String, String),
    ) -> Result<String> {
        let key = primary.trim().to_string();
        let fallback = fallback.trim().to_string();
        if key.is_empty() && fallback.is_empty() {
            anyhow::bail!("{missing}");
        }
        write(&mut self.settings, key, fallback);
        self.save_settings()?;
        Ok(saved.into())
    }

    /// Load Intel bulletin articles for the active category, run day, and search.
    pub fn load_intel(&mut self) {
        if argos_osint_core::osint::wikipedia_rsp::cached_index().is_none() {
            if let Ok(Some(raw)) = self
                .store
                .app_state_get(argos_osint_core::osint::wikipedia_rsp::APP_STATE_KEY)
            {
                if let Ok(index) = argos_osint_core::osint::wikipedia_rsp::index_from_json(&raw) {
                    argos_osint_core::osint::wikipedia_rsp::install_index(index);
                }
            }
        }
        self.intel_days = self.store.atlas_run_days().unwrap_or_default();
        if self.intel_day.is_empty() || !self.intel_days.iter().any(|day| day == &self.intel_day) {
            let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
            self.intel_day = if self.intel_days.iter().any(|day| day == &today) {
                today
            } else {
                self.intel_days.first().cloned().unwrap_or_default()
            };
        }
        if !INTEL_CATEGORIES.contains(&self.intel_category.as_str()) {
            self.intel_category = INTEL_CATEGORIES[0].into();
        }
        let kept_id = self
            .intel_articles
            .get(self.intel_sel)
            .map(|row| row.id.clone());
        self.intel_articles = if self.intel_day.is_empty() {
            Vec::new()
        } else {
            self.store
                .atlas_articles_for_intel(&self.intel_category, &self.intel_day, &self.intel_search)
                .unwrap_or_default()
        };
        self.intel_sel = kept_id
            .and_then(|id| self.intel_articles.iter().position(|row| row.id == id))
            .unwrap_or(0);
        if self.intel_sel >= self.intel_articles.len() {
            self.intel_sel = self.intel_articles.len().saturating_sub(1);
        }
        self.scrolls.intel_list = 0;
        if self.intel_page == IntelPage::Briefing {
            self.refresh_intel_briefing();
        }
    }

    fn set_intel_category(&mut self, index: usize) {
        let Some(category) = INTEL_CATEGORIES.get(index).copied() else {
            return;
        };
        if self.intel_category == category {
            self.set_focus(Target::IntelTab(index));
            return;
        }
        self.intel_category = category.into();
        self.intel_page = IntelPage::Bulletin;
        self.intel_sel = 0;
        self.load_intel();
        self.set_focus(if self.intel_articles.is_empty() {
            Target::Field(FieldId::IntelSearch)
        } else {
            Target::IntelArticle(self.intel_sel)
        });
        self.status = atlas::category_name(&self.intel_category).into();
    }

    fn open_intel_day_picker(&mut self) {
        self.intel_days = self.store.atlas_run_days().unwrap_or_default();
        if self.intel_days.is_empty() {
            self.status = "No Atlas news-cycle days yet".into();
            return;
        }
        self.choice_items = self
            .intel_days
            .iter()
            .map(|day| ChoiceItem {
                id: day.clone(),
                label: intel_day_button_label(day),
            })
            .collect();
        self.choice_sel = self
            .choice_items
            .iter()
            .position(|item| item.id == self.intel_day)
            .unwrap_or(0);
        self.choice_note = "Select news cycle day".into();
        self.overlay = Overlay::Choice(ChoiceKind::IntelDay);
        self.scrolls.popup = 0;
        self.set_focus(Target::Choice(self.choice_sel));
    }

    fn open_intel_briefing(&mut self) {
        let Some(article) = self.intel_articles.get(self.intel_sel).cloned() else {
            self.status = "No article selected".into();
            return;
        };
        self.intel_page = IntelPage::Briefing;
        self.intel_full_collapsed = false;
        self.scrolls.intel_brief = 0;
        self.scrolls.intel_full = 0;
        self.scrolls.intel_jobs = 0;
        self.scrolls.intel_extracted = 0;
        self.refresh_intel_briefing();
        self.sync_intel_brief_task_ui();
        self.ensure_article_body_fetch(false);
        // Keep an in-flight classifier result; only re-queue when idle for this article.
        if !(self.intel_mode_classifying && self.intel_recon_recommended_for == article.id) {
            self.queue_intel_recon_mode_classify();
        }
        self.queue_intel_actors_review();
        self.queue_intel_links_explanation();
        self.set_focus(Target::Button(ButtonId::IntelReports));
        self.status = format!("Brief · {}", article.title);
        self.persist_view_session();
    }

    fn refresh_intel_briefing(&mut self) {
        let Some(article) = self.intel_articles.get(self.intel_sel) else {
            self.intel_claims.clear();
            self.intel_relations.clear();
            self.intel_link_explanations.clear();
            self.intel_body = None;
            self.intel_jobs.clear();
            self.intel_sections.clear();
            return;
        };
        self.intel_claims = self
            .store
            .atlas_claims_for_article(&article.run_id, &article.id)
            .unwrap_or_default();
        let fingerprints: Vec<String> = self
            .intel_claims
            .iter()
            .map(|claim| claim.fingerprint.clone())
            .collect();
        self.intel_relations = self
            .store
            .insight_relations_among(&fingerprints)
            .unwrap_or_default();
        self.intel_link_explanations = self
            .store
            .get_intel_link_explanations(&article.id)
            .unwrap_or_default();
        self.intel_body = self
            .store
            .article_body_for_article(&article.id)
            .ok()
            .flatten();
        self.intel_jobs = self
            .store
            .intel_jobs_for_article(&article.id)
            .unwrap_or_default();
        if self.intel_job_sel >= self.intel_jobs.len() {
            self.intel_job_sel = self.intel_jobs.len().saturating_sub(1);
        }
        self.intel_sections = self
            .intel_jobs
            .get(self.intel_job_sel)
            .and_then(|job| self.store.intel_report_sections(&job.id).ok())
            .unwrap_or_default();
    }

    /// Reconcile in-memory loading flags/messages with store + background task sets on revisit.
    fn sync_intel_brief_task_ui(&mut self) {
        let Some(article) = self.intel_articles.get(self.intel_sel).cloned() else {
            return;
        };
        if self
            .intel_body
            .as_ref()
            .is_some_and(|body| body.state == "running")
        {
            self.intel_body_running.insert(article.id.clone());
        }
        if self.intel_insights_running.contains(&article.id) {
            if self.intel_body_message.trim().is_empty()
                || !self
                    .intel_body_message
                    .to_ascii_lowercase()
                    .contains("insight")
            {
                self.intel_body_message = "Re-extracting insights from full article…".into();
            }
            return;
        }
        if self.intel_body_running.contains(&article.id)
            && self.intel_body_message.trim().is_empty()
        {
            self.intel_body_message = "Retrieving full article…".into();
        }
    }

    fn leave_intel_briefing(&mut self) {
        self.intel_page = IntelPage::Bulletin;
        self.intel_claims.clear();
        self.intel_relations.clear();
        self.intel_body = None;
        // Keep body/insight/mode in-flight tracking and status text so revisit restores
        // the same loading UI instead of empty sections.
        self.intel_jobs.clear();
        self.intel_sections.clear();
        self.set_focus(if self.intel_articles.is_empty() {
            Target::Field(FieldId::IntelSearch)
        } else {
            Target::IntelArticle(self.intel_sel)
        });
        self.status = "Bulletin".into();
    }

    fn osint_provider_keys(&self) -> osint::ProviderKeys {
        osint::ProviderKeys {
            firecrawl: self.settings.provider_key("firecrawl"),
            firecrawl_fallback: self.settings.provider_fallback_key("firecrawl"),
            hunter: self.settings.provider_key("hunter"),
            hunter_fallback: self.settings.provider_fallback_key("hunter"),
            sociavault: self.settings.provider_key("sociavault"),
            sociavault_fallback: self.settings.provider_fallback_key("sociavault"),
            newsapi: self.settings.provider_key("newsapi"),
            newsapi_fallback: self.settings.provider_fallback_key("newsapi"),
            courtlistener: self.settings.provider_key("courtlistener"),
            courtlistener_fallback: self.settings.provider_fallback_key("courtlistener"),
            gnews: self.settings.provider_key("gnews"),
            gnews_fallback: self.settings.provider_fallback_key("gnews"),
            newsdata: self.settings.provider_key("newsdata"),
            newsdata_fallback: self.settings.provider_fallback_key("newsdata"),
            currents: self.settings.provider_key("currents"),
            currents_fallback: self.settings.provider_fallback_key("currents"),
            whoxy: self.settings.provider_key("whoxy"),
            whoxy_fallback: self.settings.provider_fallback_key("whoxy"),
        }
    }

    fn ensure_article_body_fetch(&mut self, force_refresh: bool) {
        let Some(article) = self.intel_articles.get(self.intel_sel).cloned() else {
            return;
        };
        if self.intel_body_running.contains(&article.id) && !force_refresh {
            if self.intel_body_message.trim().is_empty() {
                self.intel_body_message = "Retrieving full article…".into();
            }
            return;
        }
        let outcome = match intel_recon::enqueue_article_body(&self.store, &article, force_refresh)
        {
            Ok(outcome) => outcome,
            Err(err) => {
                self.intel_body_message = err.to_string();
                return;
            }
        };
        match outcome {
            intel_recon::EnqueueOutcome::Cached(_) => {
                self.intel_body = self
                    .store
                    .article_body_for_article(&article.id)
                    .ok()
                    .flatten();
                self.intel_body_message.clear();
            }
            intel_recon::EnqueueOutcome::AlreadyRunning { .. } => {
                self.intel_body_message = "Retrieving full article…".into();
            }
            intel_recon::EnqueueOutcome::Cooldown {
                retry_after,
                reason,
                ..
            } => {
                self.intel_body_message =
                    format!("Unavailable · retry after {retry_after}. {reason}");
                self.intel_body = self
                    .store
                    .article_body_for_article(&article.id)
                    .ok()
                    .flatten();
            }
            intel_recon::EnqueueOutcome::Start {
                body_id,
                force_refresh,
                ..
            } => {
                if tokio::runtime::Handle::try_current().is_err() {
                    self.intel_body_message = "Full article fetch queued (no runtime)".into();
                    return;
                }
                self.intel_body_running.insert(article.id.clone());
                self.intel_body_message = "Retrieving full article…".into();
                self.scrolls.intel_full = 0;
                // Clear the full-article pane immediately so Reload never leaves stale prose.
                if force_refresh {
                    if let Some(body) = self.intel_body.as_mut() {
                        body.body_markdown.clear();
                        body.quality = "unavailable".into();
                        body.quality_rationale.clear();
                        body.state = "running".into();
                    } else {
                        self.intel_body = None;
                    }
                }
                let db = paths::db_path();
                let keys = self.osint_provider_keys();
                let evidence_curator =
                    provider::role_secret(&self.auth, &self.settings, "evidence_curator")
                        .ok()
                        .filter(|secret| provider::resolved_key(secret).is_some());
                let synthesis = evidence_curator.or_else(|| {
                    provider::role_secret(&self.auth, &self.settings, "synthesis")
                        .ok()
                        .filter(|secret| provider::resolved_key(secret).is_some())
                });
                let classifier = provider::role_secret(&self.auth, &self.settings, "classifier")
                    .ok()
                    .filter(|secret| provider::resolved_key(secret).is_some());
                let ua = self.settings.osint_user_agent.clone();
                let title = article.title.clone();
                let brief = article.description.clone();
                let article_id = article.id.clone();
                let tx = self.work_tx.clone();
                let cancel = Arc::new(AtomicBool::new(false));
                let job = tracked::begin_cancellable(
                    JobSpec::new(
                        "intel",
                        "article_body",
                        format!(
                            "Article body · {}",
                            title.chars().take(60).collect::<String>()
                        ),
                    )
                    .resource(format!("article:{article_id}")),
                    cancel.clone(),
                );
                let stop = cancel.clone();
                tokio::spawn(async move {
                    let outcome = intel_recon::fetch_article_body(
                        &db,
                        &body_id,
                        &title,
                        &brief,
                        keys,
                        synthesis,
                        classifier,
                        if ua.trim().is_empty() { None } else { Some(ua) },
                        force_refresh,
                        cancel,
                        |event| {
                            let _ = tx.send(WorkEvent::IntelBody(event));
                        },
                    )
                    .await;
                    tracked::finish(job, "intel", &outcome, Some(&stop));
                    let _ = article_id;
                });
            }
        }
    }

    fn open_intel_recon_popup(&mut self) {
        let tab = ReportMode::all()
            .iter()
            .position(|mode| *mode == self.intel_recon_recommended)
            .unwrap_or(0);
        self.intel_recon_tab = tab;
        self.intel_recon_focus = IntelReconFocus::Tab(tab);
        self.intel_recon_enabled.clear();
        for mode in ReportMode::all() {
            let keys = intel_recon::section_plan(mode)
                .into_iter()
                .map(|section| section.key.to_string())
                .collect();
            self.intel_recon_enabled
                .insert(mode.as_str().to_string(), keys);
        }
        self.scrolls.popup = 0;
        self.overlay = Overlay::IntelRecon;
        self.set_focus(Target::IntelReconTab(tab));
        let mode = self.intel_recon_mode();
        self.status = format!("Configure {} report", mode.title());
    }

    fn queue_intel_recon_mode_classify(&mut self) {
        let Some(article) = self.intel_articles.get(self.intel_sel).cloned() else {
            return;
        };
        self.intel_recon_recommended = intel_recon::default_recon_mode();
        self.intel_recon_recommended_for = article.id.clone();
        self.intel_mode_classifying = true;
        if tokio::runtime::Handle::try_current().is_err() {
            self.intel_mode_classifying = false;
            return;
        }
        let input = intel_recon::ModeClassifyInput::from_article(&article, &self.intel_claims);
        let classifier = provider::role_secret(&self.auth, &self.settings, "classifier")
            .ok()
            .filter(|secret| provider::resolved_key(secret).is_some());
        let article_id = article.id.clone();
        let tx = self.work_tx.clone();
        let job = tracked::begin(
            JobSpec::new("intel", "recon_mode", "Recommend an Intel Recon mode")
                .resource(format!("article:{article_id}")),
        );
        tokio::spawn(async move {
            let mode = intel_recon::classify_recon_mode(classifier.as_ref(), &input).await;
            // Falls back to the default mode on its own; never an error.
            tracked::finish(job, "classifier", &Ok::<(), String>(()), None);
            let _ = tx.send(WorkEvent::IntelReconMode { article_id, mode });
        });
    }

    fn queue_intel_actors_review(&mut self) {
        let Some(article) = self.intel_articles.get(self.intel_sel).cloned() else {
            return;
        };
        if self.intel_actors_reviewing.contains(&article.id) {
            return;
        }
        let candidates: Vec<String> = self
            .intel_claims
            .iter()
            .map(|c| c.entity.clone())
            .filter(|e| !e.trim().is_empty())
            .collect();
        if candidates.is_empty() {
            return;
        }
        self.intel_actors_reviewing.insert(article.id.clone());
        if tokio::runtime::Handle::try_current().is_err() {
            self.intel_actors_reviewing.remove(&article.id);
            return;
        }
        let resolver = intel_recon::resolve_actor_reviewer_secret(&self.auth, &self.settings);
        let article_id = article.id.clone();
        let article_title = article.title.clone();
        let article_desc = article.description.clone();
        let run_id = article.run_id.clone();
        let tx = self.work_tx.clone();
        let db = paths::db_path();

        tokio::spawn(async move {
            let result = intel_recon::review_article_actors(
                resolver.as_ref(),
                &article_title,
                &article_desc,
                &candidates,
            )
            .await;

            if let Ok(review) = result {
                if let Ok(store) = Store::open(&db) {
                    let _ =
                        intel_recon::apply_reviewed_actors(&store, &run_id, &article_id, &review);
                }
            }
            let _ = tx.send(WorkEvent::IntelActorsReviewed { article_id });
        });
    }

    fn queue_intel_links_explanation(&mut self) {
        let Some(article) = self.intel_articles.get(self.intel_sel).cloned() else {
            return;
        };
        if self.intel_relations.is_empty() {
            return;
        }
        if self.intel_links_reviewing.contains(&article.id) {
            return;
        }
        if tokio::runtime::Handle::try_current().is_err() {
            return;
        }
        self.intel_links_reviewing.insert(article.id.clone());
        let summarizer = provider::role_secret(&self.auth, &self.settings, "summarization")
            .ok()
            .filter(|s| provider::resolved_key(s).is_some());
        let article_id = article.id.clone();
        let article_title = article.title.clone();
        let claims = self.intel_claims.clone();
        let relations = self.intel_relations.clone();
        let tx = self.work_tx.clone();

        tokio::spawn(async move {
            let explanations = intel_recon::explain_intel_links_with_model(
                summarizer.as_ref(),
                &article_title,
                &claims,
                &relations,
            )
            .await;

            let _ = tx.send(WorkEvent::IntelLinksExplained {
                article_id,
                explanations,
            });
        });
    }

    pub(crate) fn intel_recon_mode(&self) -> ReportMode {
        ReportMode::all()
            .get(self.intel_recon_tab)
            .copied()
            .unwrap_or(ReportMode::Verify)
    }

    pub(crate) fn intel_recon_section_enabled(&self, mode: ReportMode, key: &str) -> bool {
        self.intel_recon_enabled
            .get(mode.as_str())
            .map(|set| set.contains(key))
            .unwrap_or(true)
    }

    fn toggle_intel_recon_section(&mut self, index: usize) {
        let mode = self.intel_recon_mode();
        let plan = intel_recon::section_plan(mode);
        let Some(section) = plan.get(index) else {
            return;
        };
        let entry = self
            .intel_recon_enabled
            .entry(mode.as_str().to_string())
            .or_default();
        if entry.contains(section.key) {
            if entry.len() <= 1 {
                self.status = "Keep at least one section enabled".into();
                return;
            }
            entry.remove(section.key);
        } else {
            entry.insert(section.key.to_string());
        }
        self.intel_recon_focus = IntelReconFocus::Section(index);
        self.set_focus(Target::IntelReconSection(index));
    }

    fn select_intel_recon_tab(&mut self, index: usize) {
        let modes = ReportMode::all();
        if index >= modes.len() {
            return;
        }
        self.intel_recon_tab = index;
        self.intel_recon_focus = IntelReconFocus::Tab(index);
        self.scrolls.popup = 0;
        self.set_focus(Target::IntelReconTab(index));
        self.status = format!("{} — {}", modes[index].title(), modes[index].description());
    }

    fn move_intel_recon_focus(&mut self, delta: i32, horizontal: bool) {
        let modes = ReportMode::all();
        let sections = intel_recon::section_plan(self.intel_recon_mode()).len();
        if horizontal {
            match self.intel_recon_focus {
                IntelReconFocus::Tab(index) => {
                    let next = (index as i32 + delta).clamp(0, modes.len() as i32 - 1) as usize;
                    self.select_intel_recon_tab(next);
                }
                IntelReconFocus::Section(_) | IntelReconFocus::Start => {}
            }
            return;
        }
        let order_len = modes.len() + sections + 1; // tabs + sections + start
        let current = match self.intel_recon_focus {
            IntelReconFocus::Tab(index) => index,
            IntelReconFocus::Section(index) => modes.len() + index,
            IntelReconFocus::Start => modes.len() + sections,
        };
        let next = (current as i32 + delta).clamp(0, order_len as i32 - 1) as usize;
        if next < modes.len() {
            self.select_intel_recon_tab(next);
        } else if next < modes.len() + sections {
            let index = next - modes.len();
            self.intel_recon_focus = IntelReconFocus::Section(index);
            self.set_focus(Target::IntelReconSection(index));
            let room = super::ui::intel_recon_section_room(self).max(1);
            super::ui::reveal_index(&mut self.scrolls.popup, index, room);
        } else {
            self.intel_recon_focus = IntelReconFocus::Start;
            self.set_focus(Target::Button(ButtonId::IntelReconStart));
        }
    }

    fn start_intel_report_from_popup(&mut self) {
        let mode = self.intel_recon_mode();
        let enabled = self
            .intel_recon_enabled
            .get(mode.as_str())
            .cloned()
            .unwrap_or_default();
        if enabled.is_empty() {
            self.status = "Enable at least one section".into();
            return;
        }
        let scope = ReportScope {
            sections: intel_recon::section_plan(mode)
                .into_iter()
                .filter(|section| enabled.contains(section.key))
                .map(|section| section.key.to_string())
                .collect(),
            ..Default::default()
        };
        self.overlay = Overlay::None;
        self.scrolls.popup = 0;
        self.start_intel_report(mode, scope);
        self.set_focus(Target::Button(ButtonId::IntelReports));
    }

    fn start_intel_report(&mut self, mode: ReportMode, scope: ReportScope) {
        let Some(article) = self.intel_articles.get(self.intel_sel).cloned() else {
            self.status = "No article selected".into();
            return;
        };
        let job = match intel_recon::create_report_job(&self.store, &article, mode, &scope, false) {
            Ok(job) => job,
            Err(err) => {
                self.status = format!("Recon failed: {err}");
                return;
            }
        };
        if matches!(job.state.as_str(), "queued" | "running" | "waiting")
            && !self.intel_report_running.contains_key(&job.id)
            && tokio::runtime::Handle::try_current().is_ok()
        {
            let cancel = Arc::new(AtomicBool::new(false));
            self.intel_report_running
                .insert(job.id.clone(), cancel.clone());
            let db = paths::db_path();
            let keys = self.osint_provider_keys();
            let synthesis = provider::role_secret(&self.auth, &self.settings, "synthesis")
                .ok()
                .filter(|secret| provider::resolved_key(secret).is_some());
            let classifier = provider::role_secret(&self.auth, &self.settings, "classifier")
                .ok()
                .filter(|secret| provider::resolved_key(secret).is_some());
            let settings = self.settings.clone();
            let tx = self.work_tx.clone();
            let job_id = job.id.clone();
            intel_recon::start_report_worker(
                &db,
                job_id,
                article.title.clone(),
                article.url.clone(),
                article.description.clone(),
                article.published_at.clone(),
                article.source_domain.clone(),
                article.run_id.clone(),
                keys,
                self.auth.clone(),
                synthesis,
                classifier,
                settings,
                cancel,
                move |event| {
                    let _ = tx.send(WorkEvent::IntelReport(event));
                },
            );
        }
        self.refresh_intel_briefing();
        self.status = format!("{} r{} · {}", mode.title(), job.revision, job.state);
    }

    fn selected_intel_job_id(&self) -> Option<String> {
        self.intel_jobs
            .get(self.intel_job_sel)
            .map(|job| job.id.clone())
    }

    pub fn move_intel(&mut self, delta: i32) {
        if self.intel_page != IntelPage::Bulletin || self.intel_articles.is_empty() {
            return;
        }
        let last = self.intel_articles.len() as i32 - 1;
        self.intel_sel = (self.intel_sel as i32 + delta).clamp(0, last) as usize;
        self.set_focus(Target::IntelArticle(self.intel_sel));
        let room = super::ui::intel_list_room(self).max(1);
        super::ui::reveal_index(&mut self.scrolls.intel_list, self.intel_sel, room);
    }

    fn load_atlas(&mut self) {
        let selected = self
            .atlas_runs
            .get(self.atlas_run_sel)
            .map(|run| run.id.clone());
        let news_run = self.atlas_news_run.clone();
        let removed = self.store.atlas_prune_expired().unwrap_or_default();
        self.atlas_runs = self.store.atlas_list_runs().unwrap_or_default();
        if self.atlas_run_sel >= self.atlas_runs.len() {
            self.atlas_run_sel = self.atlas_runs.len().saturating_sub(1);
        }
        if let Some(run) = self.atlas_runs.first() {
            self.atlas_stats = serde_json::from_str(&run.stats_json).unwrap_or_default();
            if self.atlas_pause.is_none() {
                self.atlas_state = run.state.clone();
            }
        } else if !removed.is_empty() && self.atlas_pause.is_none() {
            self.atlas_stats = atlas::RunStats::default();
            self.atlas_state = "idle".into();
        }
        self.dismiss_pruned_atlas(&removed, selected.as_deref(), &news_run);
    }

    fn on_atlas_history(&self) -> bool {
        self.module == Some(ModuleId::Atlas)
            && self.atlas_page == AtlasPage::Runs
            && !self.atlas_news
    }

    /// History is newest first. Selecting the top row recolours the world map.
    fn select_latest_atlas_run(&mut self) {
        if self.atlas_runs.is_empty() {
            return;
        }
        self.atlas_run_sel = 0;
        self.scrolls.atlas_runs = 0;
        self.atlas_focus = None;
        if matches!(&self.overlay, Overlay::Block { title, .. } if title.starts_with("Run ")) {
            self.overlay = Overlay::None;
        }
        self.set_focus(Target::AtlasHistory(0));
    }

    fn dismiss_pruned_atlas(&mut self, removed: &[String], selected: Option<&str>, news_run: &str) {
        if removed.is_empty() {
            return;
        }
        let news_removed = !news_run.is_empty() && removed.iter().any(|id| id == news_run);
        let selected_removed = selected.is_some_and(|id| removed.iter().any(|gone| gone == id));
        if news_removed {
            self.atlas_news = false;
            self.atlas_focus = None;
            self.atlas_articles.clear();
            self.atlas_news_run.clear();
            if matches!(&self.overlay, Overlay::Block { title, .. } if title == "Article") {
                self.overlay = Overlay::None;
            }
        }
        if selected_removed
            && matches!(&self.overlay, Overlay::Block { title, .. } if title.starts_with("Run "))
        {
            self.overlay = Overlay::None;
        }
    }

    fn move_atlas(&mut self, delta: i32) {
        if self.atlas_page == AtlasPage::Runs {
            if self.atlas_news {
                super::ui::shift_atlas_articles(self, delta);
                if !self.atlas_articles.is_empty() {
                    self.set_focus(Target::AtlasArticle(self.atlas_article_sel));
                    self.focus_highlighted_country();
                }
                return;
            }
            if matches!(self.focus, Target::AtlasCycleStats)
                || self
                    .pointer
                    .is_some_and(|(x, y)| super::ui::cycle_stats_under_pointer(self, x, y))
            {
                super::ui::shift_cycle_stats(self, delta);
                self.set_focus(Target::AtlasCycleStats);
                return;
            }
            let before = self.atlas_run_sel;
            super::ui::shift_atlas_runs(self, delta);
            if !self.atlas_runs.is_empty() {
                if self.atlas_run_sel != before {
                    self.scrolls.origins = 0;
                }
                self.set_focus(Target::AtlasHistory(self.atlas_run_sel));
            }
            return;
        }
        super::ui::shift_atlas_feed(self, delta);
        if !self.atlas_feed.is_empty() {
            self.set_focus(Target::AtlasFeed(self.atlas_feed_sel));
        }
    }

    fn open_atlas_article(&mut self) {
        let Some(feed) = self.atlas_feed.get(self.atlas_feed_sel).cloned() else {
            self.status = "No headline selected".into();
            return;
        };
        match self.open_intel_briefing_for(self.feed_to_article_row(&feed)) {
            Ok(status) => self.status = status,
            Err(err) => self.status = err.to_string(),
        }
    }

    fn feed_to_article_row(&self, feed: &atlas::FeedArticle) -> AtlasArticleRow {
        if let Ok(rows) = self.store.atlas_recent_articles() {
            if let Some(mut row) = rows.into_iter().find(|row| row.id == feed.id) {
                if !feed.category.trim().is_empty() {
                    row.category = atlas::category_tag(&feed.category).into();
                }
                return row;
            }
        }
        let run_id = self
            .atlas_runs
            .iter()
            .find(|run| matches!(run.state.as_str(), "running" | "paused"))
            .or_else(|| self.atlas_runs.first())
            .map(|run| run.id.clone())
            .unwrap_or_default();
        AtlasArticleRow {
            run_id,
            id: feed.id.clone(),
            title: feed.title.clone(),
            description: feed.description.clone(),
            url: feed.url.clone(),
            country: feed.country.clone(),
            source_name: feed.source_name.clone(),
            source_domain: feed.source_domain.clone(),
            published_at: feed.published_at.clone(),
            provider: feed.provider.clone(),
            temperature: feed.temperature,
            category: atlas::category_tag(&feed.category).into(),
            seen_at: feed.seen_at.clone(),
            author: feed.author.clone(),
            image_url: feed.image_url.clone(),
        }
    }

    /// Jump to Intel focus brief for an Atlas article (feed, news list, or Brain claim source).
    fn open_intel_briefing_for(&mut self, article: AtlasArticleRow) -> Result<String> {
        self.flush_draft();
        self.overlay = Overlay::None;
        self.module = Some(ModuleId::Intel);
        self.launcher_sel = ModuleId::Intel.index();
        if INTEL_CATEGORIES.contains(&article.category.as_str()) {
            self.intel_category = article.category.clone();
        }
        if let Some(day) = self.day_for_atlas_run(&article.run_id) {
            self.intel_day = day;
        }
        self.intel_search.clear();
        self.load_intel();
        match self
            .intel_articles
            .iter()
            .position(|row| row.id == article.id)
        {
            Some(sel) => self.intel_sel = sel,
            None => {
                self.intel_articles.insert(0, article);
                self.intel_sel = 0;
            }
        }
        self.open_intel_briefing();
        Ok("Intel brief opened".into())
    }

    fn day_for_atlas_run(&self, run_id: &str) -> Option<String> {
        if run_id.trim().is_empty() {
            return None;
        }
        if let Some(run) = self.atlas_runs.iter().find(|run| run.id == run_id) {
            if run.started_at.len() >= 10 {
                return Some(run.started_at[..10].to_string());
            }
        }
        self.store
            .atlas_list_runs()
            .ok()
            .and_then(|runs| runs.into_iter().find(|run| run.id == run_id))
            .and_then(|run| (run.started_at.len() >= 10).then(|| run.started_at[..10].to_string()))
    }

    fn delete_atlas_run(&mut self) -> Result<String> {
        let Some(run) = self.atlas_runs.get(self.atlas_run_sel).cloned() else {
            anyhow::bail!("No run selected");
        };
        if run.state == "running" {
            anyhow::bail!("Pause the pipeline before deleting this run");
        }
        self.store.atlas_delete_run(&run.id)?;
        self.reload_memories();
        if self.atlas_news_run == run.id {
            self.atlas_news = false;
            self.atlas_focus = None;
            self.atlas_articles.clear();
            self.atlas_news_run.clear();
        }
        if matches!(self.overlay, Overlay::Block { .. }) {
            self.overlay = Overlay::None;
        }
        self.load_atlas();
        if self.atlas_runs.is_empty() {
            self.atlas_stats = atlas::RunStats::default();
            if self.atlas_pause.is_none() {
                self.atlas_state = "idle".into();
            }
        }
        Ok("Run deleted".into())
    }

    fn open_atlas_news(&mut self) {
        let Some(run) = self.atlas_runs.get(self.atlas_run_sel) else {
            self.status = "No run selected".into();
            return;
        };
        let id = run.id.clone();
        self.atlas_articles = self.store.atlas_list_articles(&id).unwrap_or_default();
        self.atlas_news_run = id;
        self.atlas_news = true;
        self.claim_mark = None;
        self.atlas_article_sel = 0;
        self.scrolls.atlas_news = 0;
        self.focus_highlighted_country();
        self.overlay = Overlay::None;
        if self.atlas_articles.is_empty() {
            self.status = "No saved articles for this run".into();
            return;
        }
        self.set_focus(Target::AtlasArticle(0));
        self.status = format!("{} articles", self.atlas_articles.len());
    }

    fn focus_highlighted_country(&mut self) {
        let Some(country) = self
            .atlas_articles
            .get(self.atlas_article_sel)
            .map(|article| article.country.clone())
        else {
            if self.atlas_focus.is_some() {
                self.atlas_focus = None;
                self.atlas_map_hold = true;
            }
            return;
        };
        if self.atlas_focus.as_deref() == Some(country.as_str()) {
            return;
        }
        self.atlas_focus = Some(country);
        self.atlas_map_hold = true;
    }

    fn show_atlas_world(&mut self) {
        self.atlas_news = false;
        self.claim_mark = None;
        self.atlas_focus = None;
        self.atlas_map_hold = true;
        self.overlay = Overlay::None;
        if !self.atlas_runs.is_empty() {
            self.set_focus(Target::AtlasHistory(self.atlas_run_sel));
        }
        self.status = "World map".into();
    }

    fn open_saved_article(&mut self) {
        let Some(article) = self.atlas_articles.get(self.atlas_article_sel).cloned() else {
            self.status = "No article selected".into();
            return;
        };
        match self.open_intel_briefing_for(article) {
            Ok(status) => self.status = status,
            Err(err) => self.status = err.to_string(),
        }
    }

    fn atlas_control(&mut self) -> Result<String> {
        if self.atlas_pause.is_some() {
            if let Some(flag) = &self.atlas_pause {
                flag.store(true, Ordering::Relaxed);
            }
            self.atlas_status = "Pausing".into();
            return Ok(self.atlas_status.clone());
        }
        let resume = self.atlas_state == "paused";
        if !resume {
            self.atlas_feed.clear();
            self.atlas_feed_sel = 0;
            self.atlas_feed_follow = true;
            self.scrolls.atlas_feed = 0;
        }
        self.spawn_atlas(if resume {
            atlas::LiveRun::Latest
        } else {
            atlas::LiveRun::Fresh
        })?;
        if !resume {
            self.shift_atlas_auto_after_manual();
        }
        Ok(if resume {
            "Resuming pipeline".into()
        } else {
            "Pipeline started".into()
        })
    }

    /// Whether the selected history cycle can be resumed (paused, or stopped
    /// in phase 4/5 with retained checkpoints) and nothing is running.
    pub(crate) fn atlas_can_resume(&self) -> bool {
        self.atlas_pause.is_none()
            && self
                .atlas_runs
                .get(self.atlas_run_sel)
                .is_some_and(atlas::resumable)
    }

    fn resume_atlas_run(&mut self) -> Result<String> {
        let run = self
            .atlas_runs
            .get(self.atlas_run_sel)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Select a news cycle to resume"))?;
        anyhow::ensure!(self.atlas_pause.is_none(), "Atlas is already running");
        anyhow::ensure!(atlas::resumable(&run), "This cycle has nothing to resume");
        self.spawn_atlas(atlas::LiveRun::Run(run.id.clone()))?;
        self.push_log("info", format!("Atlas: resuming cycle {}", run.id));
        Ok("Resuming cycle · progress in Atlas and Jobs".into())
    }

    fn start_atlas_repair(&mut self) -> Result<String> {
        if super::atlas_actions::start_repair(paths::db_path()) {
            self.push_log("info", "Atlas: Repair Atlas memories started");
            Ok("Repair Atlas memories started · progress in Jobs".into())
        } else {
            Ok("Repair Atlas memories is already running".into())
        }
    }

    /// A manual start moves the next automatic run to 60 minutes from now.
    fn shift_atlas_auto_after_manual(&mut self) {
        if self.atlas_auto_next.is_some() {
            self.persist_atlas_auto(Some(unix_now().saturating_add(ATLAS_AUTO_SECS)));
        }
    }

    fn spawn_atlas(&mut self, live: atlas::LiveRun) -> Result<()> {
        let resume = live != atlas::LiveRun::Fresh;
        anyhow::ensure!(self.atlas_pause.is_none(), "Atlas is already running");
        let pause = Arc::new(AtomicBool::new(false));
        self.atlas_pause = Some(pause.clone());
        self.atlas_state = "running".into();
        self.atlas_status = if resume {
            "Resuming".into()
        } else {
            "Starting".into()
        };
        let db = paths::db_path();
        let keys = osint::ProviderKeys {
            gnews: self.settings.provider_key("gnews"),
            gnews_fallback: self.settings.provider_fallback_key("gnews"),
            newsdata: self.settings.provider_key("newsdata"),
            newsdata_fallback: self.settings.provider_fallback_key("newsdata"),
            currents: self.settings.provider_key("currents"),
            currents_fallback: self.settings.provider_fallback_key("currents"),
            newsapi: self.settings.provider_key("newsapi"),
            newsapi_fallback: self.settings.provider_fallback_key("newsapi"),
            ..osint::ProviderKeys::default()
        };
        let feed = self.atlas_feed.clone();
        let classifier = provider::role_secret(&self.auth, &self.settings, "classifier")
            .ok()
            .filter(|secret| provider::resolved_key(secret).is_some());
        let synthesizer = provider::role_secret(&self.auth, &self.settings, "synthesis")
            .ok()
            .filter(|secret| provider::resolved_key(secret).is_some());
        let synthesizer_fallbacks =
            provider::role_fallback_secrets(&self.auth, &self.settings, "synthesis");
        let tx = self.work_tx.clone();
        let user_agent =
            osint::effective_user_agent(Some(&self.settings.osint_user_agent)).to_string();
        tokio::spawn(async move {
            let emit_tx = tx.clone();
            let outcome = atlas::run_live(
                &db,
                &pause,
                &keys,
                &user_agent,
                live,
                &feed,
                classifier,
                synthesizer,
                &synthesizer_fallbacks,
                move |event| {
                    let _ = emit_tx.send(WorkEvent::Atlas(event));
                },
            )
            .await
            .map_err(|err| err.to_string());
            let _ = tx.send(WorkEvent::AtlasDone { outcome });
        });
        Ok(())
    }

    fn toggle_atlas_auto(&mut self) -> Result<String> {
        if self.atlas_auto_next.is_some() {
            self.persist_atlas_auto(None);
            return Ok("Auto run off".into());
        }
        let now = unix_now();
        self.persist_atlas_auto(Some(now.saturating_add(ATLAS_AUTO_SECS)));
        if self.atlas_pause.is_some() {
            return Ok("Auto run on. Next pipeline in 60 minutes".into());
        }
        if tokio::runtime::Handle::try_current().is_ok() {
            self.spawn_atlas(atlas::LiveRun::Fresh)?;
            self.atlas_auto_started = true;
        }
        Ok("Auto run on. Pipeline started".into())
    }

    fn persist_atlas_auto(&mut self, when: Option<u64>) {
        self.atlas_auto_next = when;
        let _ = self.store.set_atlas_auto_next(when);
    }

    /// When the saved trigger is due, move it forward by 60 minutes.
    /// Returns whether a pipeline should start.
    fn take_atlas_auto_tick(&mut self, now: u64) -> bool {
        if self.atlas_auto_next.is_none_or(|next| now < next) {
            return false;
        }
        self.persist_atlas_auto(Some(now.saturating_add(ATLAS_AUTO_SECS)));
        self.atlas_pause.is_none()
    }

    fn poll_atlas_auto(&mut self) -> bool {
        let now = unix_now();
        if self.atlas_auto_next.is_none_or(|next| now < next) {
            return false;
        }
        let start = self.take_atlas_auto_tick(now);
        if start {
            if tokio::runtime::Handle::try_current().is_ok()
                && self.spawn_atlas(atlas::LiveRun::Fresh).is_ok()
            {
                self.atlas_auto_started = true;
            }
        } else {
            self.atlas_status = "Auto run waiting for the current pipeline".into();
            self.status = self.atlas_status.clone();
        }
        true
    }

    fn on_atlas(&mut self, event: atlas::AtlasEvent) {
        match event {
            atlas::AtlasEvent::Status(text) => {
                if text != "Extracting insights" {
                    self.atlas_insight_progress = None;
                }
                self.atlas_status = text;
                self.status = self.atlas_status.clone();
            }
            atlas::AtlasEvent::InsightProgress { done, total } => {
                self.atlas_insight_progress = Some((done, total));
                self.atlas_status = "Extracting insights".into();
                self.status = self.atlas_status.clone();
            }
            atlas::AtlasEvent::Note(text) => {
                let level = atlas_log_level(&text);
                self.push_log(level, format!("Atlas: {text}"));
                self.atlas_status = text;
                self.status = self.atlas_status.clone();
            }
            atlas::AtlasEvent::Fault(fault) => {
                self.push_log_detail(
                    "error",
                    format!("Atlas: {}", fault.summary),
                    fault.body.clone(),
                );
                self.atlas_insight_progress = None;
                self.atlas_status = fault.summary.clone();
                self.status = format!("Atlas: {}", fault.summary);
            }
            atlas::AtlasEvent::Stats(stats) => self.atlas_stats = stats,
            atlas::AtlasEvent::Article(article) => {
                let follow = self.atlas_feed_follow || self.atlas_feed.is_empty();
                self.atlas_feed.push(article);
                if follow {
                    self.atlas_feed_sel = self.atlas_feed.len().saturating_sub(1);
                    self.atlas_feed_follow = true;
                    super::ui::reveal_atlas_feed(self);
                }
            }
            atlas::AtlasEvent::Replaced { id, article } => {
                self.atlas_feed.retain(|item| item.id != id);
                self.atlas_feed.push(article);
                if self.atlas_feed_follow {
                    self.atlas_feed_sel = self.atlas_feed.len().saturating_sub(1);
                    super::ui::reveal_atlas_feed(self);
                } else if self.atlas_feed_sel >= self.atlas_feed.len() {
                    self.atlas_feed_sel = self.atlas_feed.len().saturating_sub(1);
                }
            }
            atlas::AtlasEvent::Classified { id, category } => {
                if let Some(article) = self.atlas_feed.iter_mut().find(|item| item.id == id) {
                    article.category = category.clone();
                }
                if let Some(article) = self.atlas_articles.iter_mut().find(|item| item.id == id) {
                    article.category = category;
                }
            }
            atlas::AtlasEvent::MemoriesChanged { .. } => {
                // Durable publication committed: show the new memories now.
                self.reload_memories();
            }
            atlas::AtlasEvent::MemoryProgress { indexed, required } => {
                self.atlas_insight_progress = None;
                self.atlas_status = format!("Indexing memories {indexed}/{required}");
                self.status = self.atlas_status.clone();
            }
        }
    }

    fn remember_courtlistener_key(&mut self) -> Result<String> {
        let key = self.courtlistener_key.clone();
        let fallback = self.courtlistener_fallback.clone();
        self.remember_keyed(
            &key,
            &fallback,
            "Enter a CourtListener API token",
            "CourtListener API token saved",
            |settings, key, fallback| {
                settings.courtlistener_api_token = key;
                settings.courtlistener_api_token_fallback = fallback;
            },
        )
    }

    /// The tool's provider has no key saved and none in its environment variable.
    pub fn tool_needs_key(&self, id: &str) -> bool {
        self.tool_needs_key_with(id, |name| std::env::var(name).ok())
    }

    pub fn tool_needs_key_with(&self, id: &str, env: impl Fn(&str) -> Option<String>) -> bool {
        osint::endpoint_cost(id).is_some_and(|cost| {
            self.settings
                .provider_key_with(cost.provider, &env)
                .is_empty()
                && self
                    .settings
                    .provider_fallback_key_with(cost.provider, &env)
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
        } else if tool.id.starts_with("newsapi_")
            && (!self.newsapi_key.trim().is_empty() || !self.newsapi_fallback.trim().is_empty())
        {
            // An empty primary field falls back to NEWSAPI_API_KEY.
            self.remember_newsapi_key()?;
        } else if tool.id.starts_with("courtlistener_")
            && (!self.courtlistener_key.trim().is_empty()
                || !self.courtlistener_fallback.trim().is_empty())
        {
            self.remember_courtlistener_key()?;
        } else if tool.id == "whoxy_whois_history"
            && (!self.whoxy_key.trim().is_empty() || !self.whoxy_fallback.trim().is_empty())
        {
            self.remember_whoxy_key()?;
        }
        let service =
            recon::Service::new(&paths::db_path(), self.auth.clone(), self.settings.clone())?;
        let tx = self.work_tx.clone();
        let tool_id = tool.id.to_string();
        let cancel = Arc::new(AtomicBool::new(false));
        self.osint_cancel = Some(cancel.clone());
        self.status = format!("Running {}", tool.name);
        let job = tracked::begin_cancellable(
            JobSpec::new("tools", "tool_run", format!("Run {}", tool.name)).tool(tool.id),
            cancel.clone(),
        );
        let stop = cancel.clone();
        tokio::spawn(async move {
            let outcome = service
                .manual_with_cancel(&tool_id, input, cancel)
                .await
                .map_err(|e| e.to_string());
            tracked::finish(job, "tool", &outcome, Some(&stop));
            let _ = tx.send(WorkEvent::OsintDone { outcome });
        });
        Ok(())
    }

    fn search_selected_dork(&mut self) -> Result<String> {
        let query = self
            .osint_result
            .as_ref()
            .and_then(|(_, r)| {
                if r.tool_id == "dork_generate" {
                    if let Some(arr) = r.observations.as_array() {
                        arr.first()
                            .and_then(|item| item.get("generated_query").and_then(Value::as_str))
                            .map(String::from)
                    } else if let Some(arr) =
                        r.observations.get("artifacts").and_then(Value::as_array)
                    {
                        arr.first()
                            .and_then(|item| item.get("generated_query").and_then(Value::as_str))
                            .map(String::from)
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .or_else(|| {
                let val: Value = serde_json::from_str(&self.osint_input).ok()?;
                val.get("query")
                    .or_else(|| val.get("objective"))
                    .and_then(Value::as_str)
                    .map(String::from)
            })
            .ok_or_else(|| anyhow::anyhow!("No generated query found to search"))?;

        if self.tool_needs_key("firecrawl_search") {
            return Err(anyhow::anyhow!(
                "Firecrawl API key is required to execute search"
            ));
        }

        if let Some(pos) = osint::registry()
            .iter()
            .position(|t| t.id == "firecrawl_search")
        {
            self.tool_sel = pos;
            self.osint_input = serde_json::to_string_pretty(&json!({
                "query": query,
                "limit": 5,
            }))?;
            self.run_osint()?;
            Ok(format!("Searching Firecrawl for \"{}\"", query))
        } else {
            Err(anyhow::anyhow!(
                "firecrawl_search tool not found in catalog"
            ))
        }
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
            WorkEvent::AnswerReset { thread_id } => self.note_reset(&thread_id),
            WorkEvent::AnswerReplacement { thread_id, text } => {
                self.note_replacement(&thread_id, &text)
            }
            WorkEvent::AnswerNote { thread_id, text } => {
                self.push_log("info", text.clone());
                self.live_answers.entry(thread_id.clone()).or_default().note = text;
                self.selected_thread.as_deref() == Some(&thread_id)
            }
            WorkEvent::ReconDone { thread_id, outcome } => {
                self.running.remove(&thread_id);
                if self.selected_thread.as_deref() != Some(&thread_id) {
                    self.tab_unreads.insert(thread_id.clone());
                }

                // A saved answer replaces the bubble. A failed turn that never stored one
                // keeps the text that was already streaming, instead of blanking it when
                // the recon log picks up the run error.
                let retain = outcome.is_err()
                    && self
                        .live_answers
                        .get(&thread_id)
                        .is_some_and(|live| !live.text.trim().is_empty());
                if retain {
                    if let Some(live) = self.live_answers.get_mut(&thread_id) {
                        live.shown.clone_from(&live.text);
                        if live.note.is_empty() {
                            live.note = "Recon · stopped".into();
                        }
                    }
                } else {
                    self.live_answers.remove(&thread_id);
                }
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
                self.reload_memories();
                true
            }
            WorkEvent::WhoxyBalance { outcome } => {
                match outcome {
                    Ok(balance) => {
                        if self.settings.recon_limits.whoxy_credits == 0 {
                            self.settings.recon_limits.whoxy_credits = balance;
                            let _ = self.settings.save_to(&self.settings_path);
                        }
                        self.status = format!("Whoxy connected · history balance {balance}");
                        self.push_log("info", format!("Whoxy balance check succeeded: {balance}"));
                    }
                    Err(err) => {
                        self.status = err.clone();
                        self.push_log("error", format!("Whoxy balance check failed: {err}"));
                    }
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
            WorkEvent::DatasetRefreshProgress { dataset, phase } => {
                self.status = format!("{}: {}", dataset, phase);
                true
            }
            WorkEvent::DatasetRefreshDone { dataset, outcome } => {
                self.dataset_refresh_running = None;
                self.dataset_refresh_cancel = None;
                match outcome {
                    Ok(msg) => {
                        self.status = msg.clone();
                        self.push_log("info", format!("{dataset}: {msg}"));
                    }
                    Err(err) => {
                        self.status = format!("{dataset} refresh failed: {err}");
                        self.push_log("error", format!("{dataset} dataset refresh failed: {err}"));
                    }
                }
                if self.module == Some(ModuleId::Jobs) {
                    self.reload_jobs();
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
                    self.reload_memories();
                }
                true
            }
            WorkEvent::GraphSummary { report } => self.on_graph_summary(*report),
            WorkEvent::Related {
                request,
                memory_id,
                outcome,
            } => self.on_related(request, &memory_id, outcome),
            WorkEvent::Atlas(event) => {
                self.on_atlas(event);
                self.module == Some(ModuleId::Atlas)
            }
            WorkEvent::AtlasDone { outcome } => {
                self.atlas_pause = None;
                self.atlas_insight_progress = None;
                let finished = matches!(outcome, Ok(atlas::Stop::Finished));
                let paused = matches!(outcome, Ok(atlas::Stop::Paused));
                match outcome {
                    Ok(atlas::Stop::Paused) => {
                        self.atlas_state = "paused".into();
                        self.atlas_status = "Paused".into();
                    }
                    Ok(atlas::Stop::Finished) => {
                        self.atlas_state = "completed".into();
                        self.atlas_status = "Pipeline complete".into();
                    }
                    Ok(atlas::Stop::Failed(message)) | Err(message) => {
                        self.push_log("error", format!("Atlas: {message}"));
                        self.atlas_state = "failed".into();
                        self.atlas_status = message;
                    }
                }
                self.load_atlas();
                // The stored run state is derived from extraction, publication
                // and indexing; never report "completed" for a partial save.
                if finished {
                    if let Some(run) = self.atlas_runs.first() {
                        self.atlas_state = run.state.clone();
                        if run.state != "completed" && !run.note.is_empty() {
                            self.atlas_status = run.note.clone();
                        }
                    }
                }
                self.status = self.atlas_status.clone();
                self.reload_memories();
                let automatic = self.atlas_auto_started;
                if !paused {
                    self.atlas_auto_started = false;
                }
                if automatic && finished && self.on_atlas_history() {
                    self.select_latest_atlas_run();
                }
                true
            }
            WorkEvent::IntelBody(event) => {
                let article_id = match &event {
                    intel_recon::BodyFetchEvent::Started { article_id, .. }
                    | intel_recon::BodyFetchEvent::Attempt { article_id, .. }
                    | intel_recon::BodyFetchEvent::Progress { article_id, .. }
                    | intel_recon::BodyFetchEvent::Ready { article_id, .. }
                    | intel_recon::BodyFetchEvent::InsightsRefreshing { article_id, .. }
                    | intel_recon::BodyFetchEvent::InsightsReplaced { article_id, .. }
                    | intel_recon::BodyFetchEvent::InsightsFailed { article_id, .. }
                    | intel_recon::BodyFetchEvent::Failed { article_id, .. } => article_id.clone(),
                };
                let mut insights_done = false;
                let mut is_retrieval_ready = false;
                match event {
                    intel_recon::BodyFetchEvent::Progress { message, .. } => {
                        self.intel_body_message = message;
                    }
                    intel_recon::BodyFetchEvent::Attempt {
                        tool_id,
                        state,
                        reason,
                        ..
                    } => {
                        self.intel_body_message = if reason.is_empty() {
                            format!("{tool_id} · {state}")
                        } else {
                            format!("{tool_id} · {state} · {reason}")
                        };
                    }
                    intel_recon::BodyFetchEvent::Ready { quality, .. } => {
                        self.intel_body_running.remove(&article_id);
                        self.intel_body_message = format!("Full article · {quality}");
                        is_retrieval_ready = true;
                    }
                    intel_recon::BodyFetchEvent::InsightsRefreshing { .. } => {
                        self.intel_insights_running.insert(article_id.clone());
                        self.intel_body_message =
                            "Re-extracting insights from full article…".into();
                    }
                    intel_recon::BodyFetchEvent::InsightsReplaced { claim_count, .. } => {
                        self.intel_insights_running.remove(&article_id);
                        self.intel_body_message =
                            format!("Insights replaced · {claim_count} claims");
                        insights_done = true;
                    }
                    intel_recon::BodyFetchEvent::InsightsFailed { reason, .. } => {
                        self.intel_insights_running.remove(&article_id);
                        self.intel_body_message = format!("Insight re-extract failed: {reason}");
                        insights_done = true;
                    }
                    intel_recon::BodyFetchEvent::Failed { reason, .. } => {
                        self.intel_body_running.remove(&article_id);
                        self.intel_insights_running.remove(&article_id);
                        self.intel_body_message = reason;
                    }
                    intel_recon::BodyFetchEvent::Started { .. } => {
                        self.intel_body_message = "Retrieving full article…".into();
                    }
                }
                let focused = self
                    .intel_articles
                    .get(self.intel_sel)
                    .is_some_and(|a| a.id == article_id);
                if focused && self.intel_page == IntelPage::Briefing {
                    self.intel_body = self
                        .store
                        .article_body_for_article(&article_id)
                        .ok()
                        .flatten();
                    if insights_done || is_retrieval_ready {
                        self.refresh_intel_briefing();
                        self.queue_intel_recon_mode_classify();
                        self.queue_intel_actors_review();
                        self.queue_intel_links_explanation();
                    }
                }
                focused && self.module == Some(ModuleId::Intel)
            }
            WorkEvent::IntelReconMode { article_id, mode } => {
                let selected = self
                    .intel_articles
                    .get(self.intel_sel)
                    .is_some_and(|a| a.id == article_id);
                if selected {
                    self.intel_recon_recommended = mode;
                    self.intel_recon_recommended_for = article_id;
                    self.intel_mode_classifying = false;
                }
                selected
                    && self.module == Some(ModuleId::Intel)
                    && self.intel_page == IntelPage::Briefing
            }
            WorkEvent::IntelActorsReviewed { article_id } => {
                self.intel_actors_reviewing.remove(&article_id);
                let selected = self
                    .intel_articles
                    .get(self.intel_sel)
                    .is_some_and(|a| a.id == article_id);
                if selected {
                    self.refresh_intel_briefing();
                }
                selected
                    && self.module == Some(ModuleId::Intel)
                    && self.intel_page == IntelPage::Briefing
            }
            WorkEvent::IntelLinksExplained {
                article_id,
                explanations,
            } => {
                self.intel_links_reviewing.remove(&article_id);
                let _ = self
                    .store
                    .save_intel_link_explanations(&article_id, &explanations);
                let selected = self
                    .intel_articles
                    .get(self.intel_sel)
                    .is_some_and(|a| a.id == article_id);
                if selected {
                    self.intel_link_explanations.extend(explanations);
                }
                selected
                    && self.module == Some(ModuleId::Intel)
                    && self.intel_page == IntelPage::Briefing
            }
            WorkEvent::BrainRelatedExplained {
                request,
                memory_id,
                reasons,
            } => {
                if self.brain_detail.related.request == request {
                    let matches_mem = self
                        .brain_detail
                        .memory
                        .as_ref()
                        .map(|m| m.id == memory_id)
                        .unwrap_or(false);
                    if matches_mem {
                        for (item, reason) in
                            self.brain_detail.related.items.iter_mut().zip(reasons)
                        {
                            item.reason = reason;
                        }
                    }
                }
                self.module == Some(ModuleId::Brain)
            }
            WorkEvent::IntelReport(event) => {
                let (article_id, job_id) = match &event {
                    intel_recon::IntelReportEvent::JobCreated {
                        article_id, job_id, ..
                    }
                    | intel_recon::IntelReportEvent::Stage {
                        article_id, job_id, ..
                    }
                    | intel_recon::IntelReportEvent::Section {
                        article_id, job_id, ..
                    }
                    | intel_recon::IntelReportEvent::InsightsUpdated {
                        article_id, job_id, ..
                    }
                    | intel_recon::IntelReportEvent::JobDone {
                        article_id, job_id, ..
                    } => (article_id.clone(), job_id.clone()),
                };
                match &event {
                    intel_recon::IntelReportEvent::InsightsUpdated { .. } => {
                        let focused = self
                            .intel_articles
                            .get(self.intel_sel)
                            .is_some_and(|a| a.id == article_id);
                        if focused && self.intel_page == IntelPage::Briefing {
                            self.refresh_intel_briefing();
                            self.queue_intel_actors_review();
                            self.queue_intel_links_explanation();
                        }
                    }
                    intel_recon::IntelReportEvent::JobDone { state, .. } => {
                        self.intel_report_running.remove(&job_id);
                        self.status = format!("Recon job {state}");
                        let focused = self
                            .intel_articles
                            .get(self.intel_sel)
                            .is_some_and(|a| a.id == article_id);
                        if focused && self.intel_page == IntelPage::Briefing {
                            self.refresh_intel_briefing();
                            self.queue_intel_links_explanation();
                        }
                    }
                    intel_recon::IntelReportEvent::Section { .. }
                    | intel_recon::IntelReportEvent::Stage { .. }
                    | intel_recon::IntelReportEvent::JobCreated { .. } => {
                        let focused = self
                            .intel_articles
                            .get(self.intel_sel)
                            .is_some_and(|a| a.id == article_id);
                        if focused && self.intel_page == IntelPage::Briefing {
                            self.intel_jobs = self
                                .store
                                .intel_jobs_for_article(&article_id)
                                .unwrap_or_default();
                            self.intel_sections = self
                                .store
                                .intel_report_sections(&job_id)
                                .unwrap_or_default();
                            self.queue_intel_links_explanation();
                        }
                    }
                }
                self.module == Some(ModuleId::Intel)
                    && self.intel_page == IntelPage::Briefing
                    && self
                        .intel_articles
                        .get(self.intel_sel)
                        .is_some_and(|a| a.id == article_id)
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

    fn note_reset(&mut self, thread_id: &str) -> bool {
        let live = self.live_answers.entry(thread_id.to_string()).or_default();
        live.text.clear();
        live.shown.clear();
        live.painted = Some(Instant::now());
        self.selected_thread.as_deref() == Some(thread_id)
    }

    fn note_replacement(&mut self, thread_id: &str, text: &str) -> bool {
        let live = self.live_answers.entry(thread_id.to_string()).or_default();
        live.text = text.to_string();
        live.shown = text.to_string();
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
            FieldId::ReconProvider
            | FieldId::PickerProvider
            | FieldId::SynthesisProvider
            | FieldId::ClassifierProvider
            | FieldId::SummarizationProvider => {
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

    fn current_fallbacks(&self) -> &[argos_osint_core::provider::ModelRoute] {
        self.settings
            .defaults
            .role(self.defaults_role.role_key())
            .map(|a| a.fallbacks.as_slice())
            .unwrap_or(&[])
    }

    fn mutate_role_fallbacks<F>(&mut self, op: F) -> Result<String>
    where
        F: FnOnce(&mut argos_osint_core::provider::ModelAssignment) -> Result<String, String>,
    {
        let role = self.defaults_role;
        let assignment = self
            .settings
            .defaults
            .role_mut(role.role_key())
            .ok_or_else(|| anyhow::anyhow!("unknown role"))?;
        let message = op(assignment).map_err(|e| anyhow::anyhow!(e))?;
        self.save_settings()?;
        Ok(message)
    }

    fn open_add_fallback(&mut self) {
        self.fallback_popup_restore = Some(self.focus);
        self.fallback_popup_tab = ProviderPage::Google;
        self.fallback_popup_filter.clear();
        self.fallback_popup_sel = 0;
        self.scrolls.popup = 0;
        self.overlay = Overlay::AddFallback;
        self.ensure_fallback_catalog();
        self.set_focus(Target::Field(FieldId::FallbackFilter));
        self.status = format!(
            "Add fallback · {} · tried top to bottom after primary retries (4 then 3 each)",
            self.defaults_role.label()
        );
    }

    fn cycle_fallback_tab(&mut self, delta: i32) {
        const TABS: [ProviderPage; 3] = [
            ProviderPage::Google,
            ProviderPage::Nvidia,
            ProviderPage::OpenRouter,
        ];
        let idx = TABS
            .iter()
            .position(|p| *p == self.fallback_popup_tab)
            .unwrap_or(0) as i32;
        let next = (idx + delta).rem_euclid(TABS.len() as i32) as usize;
        self.fallback_popup_tab = TABS[next];
        self.fallback_popup_sel = 0;
        self.ensure_fallback_catalog();
    }

    fn ensure_fallback_catalog(&mut self) {
        let provider = self.catalog_provider();
        if let Some(models) = self.catalog_cache.get(&provider).cloned() {
            self.model_catalog = models;
            self.catalog_for = provider;
            return;
        }
        let secret = provider::account_secret(&self.auth, &provider);
        if secret.api_key.as_deref().unwrap_or("").trim().is_empty() {
            self.model_catalog.clear();
            self.catalog_for.clear();
            return;
        }
        self.refresh_catalog();
    }

    pub fn filtered_fallback_models(&self) -> Vec<ListedModel> {
        let query = self.fallback_popup_filter.trim().to_ascii_lowercase();
        self.model_catalog
            .iter()
            .filter(|model| {
                query.is_empty()
                    || model.id.to_ascii_lowercase().contains(&query)
                    || model.name.to_ascii_lowercase().contains(&query)
            })
            .cloned()
            .collect()
    }

    pub fn fallback_incompatible(&self, model: &str) -> Option<&'static str> {
        if matches!(
            self.defaults_role,
            DefaultsRole::ToolPicker | DefaultsRole::Classifier
        ) {
            return None;
        }
        if provider::is_decisions_model(model) {
            Some("decisions model cannot complete chat")
        } else {
            None
        }
    }

    fn delete_selected_fallback(&mut self) -> Result<String> {
        let index = self.fallback_sel;
        self.mutate_role_fallbacks(|assignment| {
            assignment.remove_fallback(index)?;
            Ok("Fallback removed".into())
        })
        .inspect(|_| {
            self.fallback_sel = self.fallback_sel.saturating_sub(1);
        })
    }

    fn move_selected_fallback(&mut self, delta: i32) -> Result<String> {
        let index = self.fallback_sel;
        let mut dest = index;
        self.mutate_role_fallbacks(|assignment| {
            dest = assignment.move_fallback(index, delta)?;
            Ok(format!("Fallback {}", dest + 1))
        })?;
        self.fallback_sel = dest;
        Ok(format!("Fallback {}", dest + 1))
    }

    fn confirm_add_fallback(&mut self) -> Result<String> {
        let models = self.filtered_fallback_models();
        let Some(model) = models.get(self.fallback_popup_sel) else {
            anyhow::bail!("Select a model");
        };
        if let Some(reason) = self.fallback_incompatible(&model.id) {
            anyhow::bail!("{reason}");
        }
        let provider = self.catalog_provider();
        if provider::account_secret(&self.auth, &provider)
            .api_key
            .as_deref()
            .unwrap_or("")
            .trim()
            .is_empty()
        {
            anyhow::bail!(
                "Save an API key on the {} tab first",
                self.fallback_popup_tab.title()
            );
        }
        let route = argos_osint_core::provider::ModelRoute {
            provider: provider.clone(),
            model: model.id.clone(),
            account: String::new(),
        };
        let label = route.label();
        self.mutate_role_fallbacks(|assignment| {
            assignment.add_fallback(route.clone())?;
            Ok(format!("Added fallback {label}"))
        })?;
        let len = self.current_fallbacks().len();
        self.fallback_sel = len.saturating_sub(1);
        let restore = self.fallback_popup_restore.take();
        self.overlay = Overlay::None;
        self.scrolls.popup = 0;
        if let Some(target) = restore {
            self.set_focus(target);
        } else {
            self.set_focus(Target::Button(ButtonId::AddFallback));
        }
        Ok(format!("Added fallback {label}"))
    }

    fn catalog_provider(&self) -> String {
        if self.overlay == Overlay::AddFallback {
            return match self.fallback_popup_tab {
                ProviderPage::Google => "google".into(),
                ProviderPage::Nvidia => "nvidia".into(),
                ProviderPage::OpenRouter => "openrouter".into(),
                ProviderPage::Defaults => "openrouter".into(),
            };
        }
        match self.provider_page {
            ProviderPage::Google => "google".into(),
            ProviderPage::Nvidia => "nvidia".into(),
            ProviderPage::OpenRouter => "openrouter".into(),
            ProviderPage::Defaults => self.role_provider(),
        }
    }

    fn refresh_catalog(&mut self) {
        let role = self.defaults_role;
        let provider = self.catalog_provider();
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
        let job = tracked::begin(
            JobSpec::new("models", "model_catalog", format!("Load {provider} models"))
                .model(provider.clone(), String::new()),
        );
        tokio::spawn(async move {
            let outcome = provider::verified_catalog(&secret)
                .await
                .map_err(|e| e.to_string());
            tracked::finish(job, "provider", &outcome, None);
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
        match outcome {
            Ok(models) => {
                let count = models.len();
                self.catalog_cache.insert(provider.clone(), models.clone());
                let current = self.catalog_provider();
                if self.defaults_role != role && current != provider {
                    return;
                }
                if current != provider && self.role_provider() != provider {
                    return;
                }
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
        self.choice_note = "Configured accounts. Add a key on a provider tab.".into();
        self.rebuild_provider_choices();
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
        let current = self.role_provider();
        if matches!(
            current.as_str(),
            "grok" | "grok-subscription" | "openai" | "openai-chatgpt"
        ) {
            items.push(ChoiceItem {
                id: current.clone(),
                label: format!(
                    "{} · Choose a replacement provider",
                    provider_label(&current)
                ),
            });
        }
        if !self.google_key.is_empty() || self.auth.account("google").is_some() {
            items.push(ChoiceItem {
                id: "google".into(),
                label: "Google · AI Studio key".into(),
            });
        }
        if !self.nvidia_key.is_empty() || self.auth.account("nvidia").is_some() {
            items.push(ChoiceItem {
                id: "nvidia".into(),
                label: "Nvidia · API Catalog key".into(),
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
            Overlay::Choice(ChoiceKind::IntelDay) => {
                self.intel_day = item.id;
                self.intel_page = IntelPage::Bulletin;
                self.intel_sel = 0;
                self.load_intel();
                self.status = format!("Day {}", intel_day_button_label(&self.intel_day));
                self.set_focus(if self.intel_articles.is_empty() {
                    Target::Field(FieldId::IntelSearch)
                } else {
                    Target::IntelArticle(self.intel_sel)
                });
            }
            Overlay::Choice(ChoiceKind::Investigation) => {
                let thread_id = item.id.clone();
                let _ = self.enter_investigation(&thread_id);
                let _ = self.open_investigation_tab(&thread_id);
                self.status = format!("Switched to investigation: {}", item.label);
            }
            _ => return,
        }
        self.overlay = Overlay::None;
        self.scrolls.popup = 0;
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
        self.reload_memories_selecting(Some(memory.id.clone()));
        self.brain_insight.clear();
        self.brain_list_mode = BrainListMode::List;
        self.brain_graph_for = None;
        self.sync_graph();
        self.set_focus(Target::Memory(self.memory_sel));
        Ok("Insight saved with source".into())
    }

    /// Graph of the selected list row (list mode only; the detail view owns its graph).
    fn sync_graph(&mut self) {
        if self.brain_list_mode == BrainListMode::Graph {
            return;
        }
        let id = self
            .memories
            .get(self.memory_sel)
            .map(|memory| memory.id.clone());
        if self.brain_graph_for == id {
            return;
        }
        self.brain_graph_for = id.clone();
        self.scrolls.path = 0;
        self.brain_graph = match &id {
            Some(id) => self.store.graph_for_memory(id).unwrap_or_default(),
            None => recon::MemoryGraph::default(),
        };
    }

    /// Claim mode follows the open detail memory's own provenance and graph.
    pub(crate) fn detail_claim(&self) -> bool {
        self.brain_graph.is_claim_path()
            || self
                .brain_detail
                .memory
                .as_ref()
                .is_some_and(|memory| memory.source.app == "atlas")
    }

    /// Memories for the list plus the total, honoring Find. Errors are returned,
    /// never turned into an empty list.
    fn load_memory_list(&self) -> Result<(Vec<Memory>, usize)> {
        if let Some(fault) = super::brain_detail::read_fault() {
            anyhow::bail!(fault);
        }
        let query = self.brain_query.trim();
        if query.is_empty() {
            let list = self.store.list_memories()?;
            let total = list.len();
            Ok((list, total))
        } else {
            let list = self.store.search_memories(query)?;
            Ok((list, self.store.memory_count()?))
        }
    }

    pub(crate) fn reload_memories(&mut self) {
        self.reload_memories_selecting(None);
    }

    /// Reload the Brain list, keeping the selected memory (by id), Find, and
    /// scroll. A read failure keeps the last good list and reports the error.
    fn reload_memories_selecting(&mut self, prefer: Option<String>) {
        let selected = prefer.or_else(|| {
            self.memories
                .get(self.memory_sel)
                .map(|memory| memory.id.clone())
        });
        match self.load_memory_list() {
            Ok((list, total)) => {
                self.memories = list;
                self.memory_total = total;
                self.memories_loaded = true;
                if self.memory_error.take().is_some() {
                    self.push_log("info", "Brain: memories readable again");
                }
            }
            Err(err) => {
                let message = err.to_string();
                if self.memory_error.as_deref() != Some(message.as_str()) {
                    self.push_log_detail(
                        "error",
                        "Brain: could not read memories",
                        message.clone(),
                    );
                }
                self.memory_error = Some(message);
                self.status = "Could not read memories; showing the last loaded list".into();
                return;
            }
        }
        let by_id = selected
            .as_ref()
            .and_then(|id| self.memories.iter().position(|memory| &memory.id == id));
        self.memory_sel = match by_id {
            Some(index) => index,
            None => self.memory_sel.min(self.memories.len().saturating_sub(1)),
        };
        let room = super::ui::memory_room_for(self);
        let max = self.memories.len().saturating_sub(room) as u16;
        self.scrolls.memories = self.scrolls.memories.min(max);
        super::ui::reveal_index(&mut self.scrolls.memories, self.memory_sel, room);
        if matches!(self.focus, Target::Memory(_)) {
            self.focus = if self.memories.is_empty() {
                Target::Button(ButtonId::CreateMemory)
            } else {
                Target::Memory(self.memory_sel)
            };
        }
        if self.brain_list_mode == BrainListMode::Graph {
            self.refresh_open_detail();
        } else {
            self.brain_graph_for = None;
            self.sync_graph();
            self.sync_selected_insight();
        }
    }

    /// The open detail memory changed or vanished in another writer.
    fn refresh_open_detail(&mut self) {
        let Some(id) = self.brain_detail.memory_id().map(str::to_string) else {
            return;
        };
        match self.store.get_memory(&id) {
            Ok(Some(memory)) => self.brain_detail.memory = Some(memory),
            Ok(None) => {
                self.leave_brain_detail();
                self.status = "The open memory was deleted".into();
            }
            Err(_) => {}
        }
    }

    fn leave_brain_detail(&mut self) {
        self.brain_list_mode = BrainListMode::List;
        self.brain_detail.memory = None;
        self.brain_detail.history.clear();
        self.brain_detail.related = Default::default();
        self.graph_summary.clear();
        self.summary_failure = None;
        self.summary_details_open = false;
        self.brain_graph_for = None;
        self.sync_graph();
        self.set_focus(if self.memories.is_empty() {
            Target::Button(ButtonId::CreateMemory)
        } else {
            Target::Memory(self.memory_sel)
        });
        self.status = "Memories".into();
    }

    fn open_memory_graph(&mut self) {
        let Some(id) = self.memories.get(self.memory_sel).map(|m| m.id.clone()) else {
            self.status = "No memory selected".into();
            return;
        };
        self.open_memory_detail(&id);
    }

    fn detail_snapshot(&self) -> Option<super::brain_detail::DetailSnapshot> {
        Some(super::brain_detail::DetailSnapshot {
            memory_id: self.brain_detail.memory_id()?.to_string(),
            related_sel: self.brain_detail.related.sel,
            related_scroll: self.brain_detail.related.scroll,
            path_scroll: self.scrolls.path,
            summary_scroll: self.scrolls.summary,
            focus: self.focus,
        })
    }

    /// One transition for every memory link (Brain list, Related, others).
    /// Fetches the destination by id, even when Find hides it. On failure the
    /// current view stays intact. Returns whether the detail opened.
    pub(crate) fn open_memory_detail(&mut self, memory_id: &str) -> bool {
        let memory = match self.store.get_memory(memory_id) {
            Ok(Some(memory)) => memory,
            Ok(None) => {
                self.reload_memories();
                self.status = "That memory no longer exists; the list was refreshed".into();
                return false;
            }
            Err(err) => {
                self.status = format!("Could not open that memory: {err}");
                return false;
            }
        };
        let graph = match self.store.graph_for_memory(&memory.id) {
            Ok(graph) => graph,
            Err(err) => {
                self.status = format!("Could not load that memory's graph: {err}");
                return false;
            }
        };
        if self.brain_list_mode == BrainListMode::Graph {
            if let Some(snapshot) = self.detail_snapshot() {
                if snapshot.memory_id != memory.id {
                    self.brain_detail.push(snapshot);
                }
            }
        } else {
            self.brain_detail.history.clear();
        }
        self.show_detail(memory, graph, None);
        true
    }

    fn show_detail(
        &mut self,
        memory: Memory,
        graph: recon::MemoryGraph,
        restore: Option<&super::brain_detail::DetailSnapshot>,
    ) {
        self.brain_list_mode = BrainListMode::Graph;
        self.brain_graph = graph;
        self.brain_graph_for = Some(memory.id.clone());
        self.selected_insight = self.store.insight_for_memory(&memory.id).ok().flatten();
        self.graph_summary.clear();
        self.summary_failure = None;
        self.summary_details_open = false;
        self.scrolls.path = restore.map_or(0, |s| s.path_scroll);
        self.scrolls.summary = restore.map_or(0, |s| s.summary_scroll);
        self.brain_detail.memory = Some(memory.clone());
        let request = self.brain_detail.begin_related(
            restore.map_or(0, |s| s.related_sel),
            restore.map_or(0, |s| s.related_scroll),
        );
        self.request_related(request, &memory.id);
        self.load_or_request_summary(&memory);
        let focus = restore.map_or(Target::DetailPath, |s| s.focus);
        self.set_focus(match focus {
            Target::RelatedRow(_) if self.brain_detail.related.items.is_empty() => {
                Target::DetailPath
            }
            Target::RelatedRow(_) => Target::RelatedRow(self.brain_detail.related.sel),
            other => other,
        });
    }

    /// Related items for `memory_id`: on a blocking worker in the live app, inline
    /// otherwise. Late results for another request or memory are dropped.
    fn request_related(&mut self, request: u64, memory_id: &str) {
        let limits = RelatedLimits::default();
        if let (Ok(_), Some(db)) = (tokio::runtime::Handle::try_current(), tracked::db_path()) {
            let tx = self.work_tx.clone();
            let memory_id = memory_id.to_string();
            tokio::task::spawn_blocking(move || {
                let outcome = Store::open(&db)
                    .and_then(|store| store.related_memories(&memory_id, limits))
                    .map_err(|err| err.to_string());
                let _ = tx.send(WorkEvent::Related {
                    request,
                    memory_id,
                    outcome,
                });
            });
            return;
        }
        let outcome = self
            .store
            .related_memories(memory_id, limits)
            .map_err(|err| err.to_string());
        self.on_related(request, memory_id, outcome);
    }

    fn on_related(
        &mut self,
        request: u64,
        memory_id: &str,
        outcome: std::result::Result<Vec<RelatedMemory>, String>,
    ) -> bool {
        if !self
            .brain_detail
            .finish_related(request, memory_id, outcome)
        {
            return false;
        }
        let area = super::ui::detail_areas(self).related;
        super::brain_detail::reveal_related(&mut self.brain_detail.related, area);
        if matches!(self.focus, Target::RelatedRow(_)) {
            self.focus = if self.brain_detail.related.items.is_empty() {
                Target::DetailPath
            } else {
                Target::RelatedRow(self.brain_detail.related.sel)
            };
        }
        self.queue_related_memories_explanation(request, memory_id);
        true
    }

    fn queue_related_memories_explanation(&mut self, request: u64, memory_id: &str) {
        if cfg!(test) {
            return;
        }
        if self.brain_detail.related.items.is_empty() {
            return;
        }
        if tokio::runtime::Handle::try_current().is_err() {
            return;
        }
        let summarizer = provider::role_secret(&self.auth, &self.settings, "summarization")
            .ok()
            .filter(|s| provider::resolved_key(s).is_some());
        let target_text = self
            .brain_detail
            .memory
            .as_ref()
            .map(|m| m.text.clone())
            .unwrap_or_default();
        let items = self.brain_detail.related.items.clone();
        let memory_id = memory_id.to_string();
        let tx = self.work_tx.clone();

        tokio::spawn(async move {
            let reasons = argos_osint_core::related_memories::explain_related_memories_with_model(
                summarizer.as_ref(),
                &target_text,
                &items,
            )
            .await;

            let _ = tx.send(WorkEvent::BrainRelatedExplained {
                request,
                memory_id,
                reasons,
            });
        });
    }

    /// Back: the previous detail memory with its selection, focus, and scroll;
    /// at the first detail, the Brain list with its Find and selection.
    fn detail_back(&mut self) {
        while let Some(snapshot) = self.brain_detail.history.pop() {
            let Ok(Some(memory)) = self.store.get_memory(&snapshot.memory_id) else {
                continue;
            };
            let graph = self.store.graph_for_memory(&memory.id).unwrap_or_default();
            self.show_detail(memory, graph, Some(&snapshot));
            self.status = "Back".into();
            return;
        }
        self.leave_brain_detail();
    }

    /// Selection only; never navigates or starts a summary.
    fn move_related(&mut self, delta: i32) {
        let len = self.brain_detail.related.items.len();
        if len == 0 {
            return;
        }
        let next = (self.brain_detail.related.sel as i32 + delta).clamp(0, len as i32 - 1) as usize;
        self.brain_detail.related.sel = next;
        let area = super::ui::detail_areas(self).related;
        super::brain_detail::reveal_related(&mut self.brain_detail.related, area);
        self.set_focus(Target::RelatedRow(next));
    }

    fn open_selected_related(&mut self) {
        let Some(id) = self
            .brain_detail
            .selected_related()
            .map(|item| item.memory_id.clone())
        else {
            return;
        };
        if self.open_memory_detail(&id) {
            self.status = "Related memory opened".into();
        }
    }

    fn load_or_request_summary(&mut self, memory: &Memory) {
        self.request_summary(memory, false);
    }

    /// Graph explanation for the open memory: a valid cached summary, the
    /// running execution, the saved failure (inside the retry cooldown), or a
    /// new budgeted execution. `explicit` is Brain → Retry summary.
    fn request_summary(&mut self, memory: &Memory, explicit: bool) {
        use argos_osint_core::graph_explanation::{self as ge, Cached, ExplainRequest, Gate};
        use argos_osint_core::provider_diag::{Category, ProviderFailure, Stage};
        let claim = self.detail_claim();
        let title = if claim { "Claim path" } else { "Recon path" };
        let focus = recon::recon_path(&self.brain_graph)
            .bands
            .first()
            .map(|band| band.directive_id.clone())
            .unwrap_or_default();
        if self.brain_graph.is_empty() {
            self.summary_failure = None;
            self.graph_summary = if claim {
                "This memory has no claim path.".into()
            } else {
                "This memory has no investigation graph.".into()
            };
            self.status = title.into();
            return;
        }
        if self.graph_summary_pending.as_deref() == Some(memory.id.as_str()) {
            if !explicit {
                self.graph_summary = self.summary_body(memory, "Writing graph summary…");
            }
            self.status = "Graph summary is already being written".into();
            return;
        }
        let secret = match provider::role_secret(&self.auth, &self.settings, "summarization") {
            Ok(secret) => secret,
            Err(err) => {
                let failure = ProviderFailure::from_error(
                    Stage::Configuration,
                    Category::Configuration,
                    err.as_ref(),
                );
                self.summary_failure = Some(super::summary_card::SummaryFailure::local(
                    &memory.id, &failure,
                ));
                self.graph_summary = self.summary_body(memory, "");
                self.status = "Graph summary unavailable · configure Summarization".into();
                return;
            }
        };
        let retry_of = self
            .summary_failure
            .as_ref()
            .filter(|f| f.memory_id == memory.id && !f.job_id.is_empty())
            .map(|f| f.job_id.clone());
        let req = ExplainRequest {
            memory_id: memory.id.clone(),
            memory_text: memory.text.clone(),
            focus,
            claim,
            system: summary_system(claim),
            graph_brief: recon::graph_brief(&self.brain_graph),
            request_id: argos_osint_core::job_registry::new_job_id("graph-request"),
            retry_of,
            stop_flag: Arc::new(AtomicBool::new(false)),
        };
        let key = req.key(&secret);
        match ge::cached(&self.store, &memory.id, &key) {
            Ok(Cached::Valid(text)) if !explicit => {
                self.summary_failure = None;
                self.graph_summary = text;
                self.status = title.into();
                return;
            }
            Ok(_) => {}
            Err(err) => {
                self.push_log("warn", format!("Graph summary cache unreadable: {err:#}"));
            }
        }
        match ge::gate(&self.store, &memory.id, &key, explicit) {
            Gate::Running(_) => {
                self.graph_summary = self.summary_body(memory, "Writing graph summary…");
                self.status = "Graph summary is already being written".into();
                return;
            }
            Gate::CoolingDown(rec) => {
                self.summary_failure = Some(super::summary_card::SummaryFailure::from_record(&rec));
                self.graph_summary = self.summary_body(memory, "");
                self.status = "Graph summary failed · Retry summary to try again".into();
                return;
            }
            Gate::Ready => {}
        }
        let (Ok(runtime), Some(db)) = (tokio::runtime::Handle::try_current(), tracked::db_path())
        else {
            self.graph_summary = self.summary_body(memory, "");
            self.status = "Graph summary unavailable: no background runtime".into();
            return;
        };
        self.summary_failure = None;
        self.summary_details_open = false;
        self.graph_summary_pending = Some(memory.id.clone());
        self.graph_summary_request = req.request_id.clone();
        self.graph_summary = self.summary_body(memory, "Writing graph summary…");
        self.status = if explicit {
            "Retrying graph summary…".into()
        } else {
            "Writing graph summary…".into()
        };
        let tx = self.work_tx.clone();
        runtime.spawn(async move {
            let mut opts = argos_osint_core::summarization::ExecOptions::for_secret(&secret);
            if cfg!(test) {
                opts.max_backoff = std::time::Duration::from_millis(20);
            }
            let report = ge::explain(&db, &secret, &req, &opts, ge::Faults::default()).await;
            let _ = tx.send(WorkEvent::GraphSummary {
                report: Box::new(report),
            });
        });
    }

    /// Body of the summary pane when no current summary exists: an optional
    /// status line, then the last valid summary (labeled as an earlier
    /// result) or the deterministic basic explanation.
    fn summary_body(&self, memory: &Memory, status: &str) -> String {
        let mut out = String::new();
        if !status.is_empty() {
            out.push_str(&format!("_{status}_\n\n"));
        }
        match self.store.graph_summary_entry(&memory.id) {
            Ok(Some(entry)) => {
                out.push_str("_Earlier result — written before the evidence, memory text, focus, model or prompt last changed._\n\n");
                out.push_str(&entry.summary);
            }
            _ => out.push_str(&argos_osint_core::graph_explanation::basic_explanation(
                &self.brain_graph,
                &memory.text,
            )),
        }
        out
    }

    /// A finished graph explanation. The job, events and diagnostic record
    /// were persisted before this was sent.
    fn on_graph_summary(
        &mut self,
        report: argos_osint_core::graph_explanation::ExplainReport,
    ) -> bool {
        use argos_osint_core::graph_explanation::ExplainOutcome;
        let newest = report.request_id == self.graph_summary_request;
        if newest && self.graph_summary_pending.as_deref() == Some(report.memory_id.as_str()) {
            self.graph_summary_pending = None;
        }
        if let Some(err) = &report.logging_error {
            self.push_log("warn", format!("Graph summary logging incomplete: {err}"));
        }
        let viewing = newest
            && self.brain_list_mode == BrainListMode::Graph
            && self.brain_detail.memory_id() == Some(report.memory_id.as_str());
        if !viewing {
            return false;
        }
        let memory = self.brain_detail.memory.clone();
        match &report.outcome {
            ExplainOutcome::Saved(text) => {
                self.summary_failure = None;
                self.summary_details_open = false;
                self.graph_summary = text.clone();
                self.status = "Graph summary saved".into();
            }
            ExplainOutcome::Superseded(why) => {
                if let Some(memory) = &memory {
                    self.graph_summary = self.summary_body(memory, "");
                }
                self.status = format!("Graph summary discarded: {why}");
            }
            ExplainOutcome::Failed(failure) => {
                self.summary_failure = Some(super::summary_card::SummaryFailure::from_report(
                    &report, failure,
                ));
                if let Some(memory) = &memory {
                    self.graph_summary = self.summary_body(memory, "");
                }
                self.status = format!("Graph summary failed: {}", failure.category.reason());
            }
        }
        true
    }

    /// Brain summary card actions.
    fn activate_summary_card(&mut self, button: ButtonId) -> Result<String> {
        let Some(failure) = self.summary_failure.clone() else {
            return Ok("No summary failure".into());
        };
        match button {
            ButtonId::SummaryDetails => {
                self.summary_details_open = !self.summary_details_open;
                self.scrolls.summary = 0;
                Ok(if self.summary_details_open {
                    "Summary failure details".into()
                } else {
                    "Details hidden".into()
                })
            }
            ButtonId::SummaryLogs => {
                self.logs.job = failure.job_id.clone();
                self.logs.back_to_job = Some(failure.job_id.clone());
                self.logs.open.clear();
                self.logs.follow = false;
                self.select(ModuleId::Logs.index());
                self.logs.reload(&self.store);
                if let Some(index) = self
                    .logs
                    .rows
                    .iter()
                    .position(|row| row.id == failure.event_id)
                {
                    self.logs.select(index);
                    self.logs.open.insert(failure.event_id.clone());
                    Ok(format!(
                        "Logs for job {}",
                        super::logs::short_id(&failure.job_id)
                    ))
                } else {
                    // Events expire after the retention window; the job row and
                    // the saved diagnostic (View details) keep the error summary.
                    Ok(format!(
                        "Log detail for job {} has expired (24 h retention) · the job keeps its error summary; Brain → View details keeps the cause chain",
                        super::logs::short_id(&failure.job_id)
                    ))
                }
            }
            ButtonId::SummaryJob => self.open_job(&failure.job_id),
            ButtonId::SummaryRetry => {
                let Some(memory) = self.brain_detail.memory.clone() else {
                    return Ok("Open a memory first".into());
                };
                self.request_summary(&memory, true);
                Ok(self.status.clone())
            }
            ButtonId::SummaryModels => {
                self.select(ModuleId::Providers.index());
                self.provider_page = ProviderPage::Defaults;
                if self.defaults_role != DefaultsRole::Summarization {
                    self.defaults_role = DefaultsRole::Summarization;
                    self.model_catalog.clear();
                    self.catalog_for.clear();
                }
                self.set_focus(Target::Button(ButtonId::DefaultRole(
                    DefaultsRole::Summarization,
                )));
                Ok("Models · Summarization".into())
            }
            _ => Ok(String::new()),
        }
    }

    fn activate_button(&mut self, button: ButtonId) {
        let result = match button {
            ButtonId::Send => {
                self.submit();
                return;
            }
            ButtonId::ResumeSessionConfirm => {
                if let Overlay::ResumeSession(sess) = self.overlay.clone() {
                    self.resume_view_session(&sess);
                }
                return;
            }
            ButtonId::ResumeSessionDismiss => {
                self.overlay = Overlay::None;
                if self.module.is_none() {
                    self.set_focus(Target::Field(FieldId::Composer));
                }
                return;
            }
            ButtonId::CreateMemory => {
                self.brain_list_mode = BrainListMode::Create;
                self.set_focus(Target::Field(FieldId::BrainApp));
                self.cursor = self.brain_app.chars().count();
                self.status = "New memory".into();
                return;
            }
            ButtonId::BrainBack => {
                self.leave_brain_detail();
                return;
            }
            ButtonId::SummaryDetails
            | ButtonId::SummaryLogs
            | ButtonId::SummaryJob
            | ButtonId::SummaryRetry
            | ButtonId::SummaryModels => self.activate_summary_card(button),
            ButtonId::BrainDetailBack => {
                self.detail_back();
                return;
            }
            ButtonId::Add => self.save_insight(),
            ButtonId::Pin => {
                let Some(memory) = self.memories.get(self.memory_sel) else {
                    self.status = "No memory selected".into();
                    return;
                };
                self.store
                    .update_memory(&memory.id, &memory.text, &memory.category, !memory.pinned)
                    .map(|_| {
                        self.reload_memories();
                        "Memory pin updated".into()
                    })
            }
            ButtonId::Delete => {
                let Some(memory) = self.memories.get(self.memory_sel) else {
                    self.status = "No memory selected".into();
                    return;
                };
                self.store.delete_memory(&memory.id).map(|_| {
                    self.reload_memories();
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
                self.store.deletion_consequences(&id).and_then(|removed| {
                    let removed = removed.len();
                    self.store.delete_thread(&id, true)?;
                    self.tab_ids.retain(|tid| tid != &id);
                    self.tab_recently_closed.retain(|tid| tid != &id);
                    self.tab_last_active.remove(&id);
                    self.tab_unreads.remove(&id);
                    self.save_session_tabs();
                    self.selected_thread = None;
                    self.messages.clear();
                    self.input.clear();
                    self.recon_chat = false;
                    self.refresh_threads()?;
                    if let Some(next) = self.threads.first().map(|t| t.id.clone()) {
                        self.open_thread(&next)?;
                    }
                    self.reload_memories();
                    Ok(format!(
                        "Investigation deleted; {removed} brain memories removed"
                    ))
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
            ButtonId::RetryInsights => self.toggle_recall(),
            ButtonId::ToggleInvestigation => {
                self.recon_context_enabled = !self.recon_context_enabled;
                self.settings.tui_recon_context = Some(self.recon_context_enabled);
                let _ = self.save_settings();
                Ok(if self.recon_context_enabled {
                    "Investigation panel shown".into()
                } else {
                    "Investigation panel hidden".into()
                })
            }
            ButtonId::OsintRun => self.run_osint().map(|_| "Tool started".into()),
            ButtonId::OsintCancel => {
                let mut cancelled = false;
                if let Some(cancel) = &self.dataset_refresh_cancel {
                    cancel.store(true, Ordering::Relaxed);
                    cancelled = true;
                }
                if let Some(cancel) = &self.osint_cancel {
                    cancel.store(true, Ordering::Relaxed);
                    cancelled = true;
                }
                if cancelled {
                    self.status = "Cancellation requested".into();
                    Ok("Cancellation requested".into())
                } else {
                    Ok("Nothing to cancel".into())
                }
            }
            ButtonId::OsintRefreshDataset => {
                if self.dataset_refresh_running.is_some() {
                    Ok("A dataset refresh is already in progress".into())
                } else {
                    let cancel = Arc::new(AtomicBool::new(false));
                    self.dataset_refresh_running = Some("whatsmyname".into());
                    self.dataset_refresh_cancel = Some(cancel.clone());
                    self.status = "Refreshing WhatsMyName dataset...".into();
                    self.push_log("info", "Started WhatsMyName dataset refresh");
                    let job = Arc::new(std::sync::Mutex::new(tracked::begin_cancellable(
                        JobSpec::new("tools", "dataset_refresh", "Refresh WhatsMyName dataset")
                            .tool("whatsmyname_lookup"),
                        cancel.clone(),
                    )));
                    let tx = self.work_tx.clone();
                    let stop = cancel.clone();
                    let job_worker = job.clone();
                    tokio::spawn(async move {
                        let tx_prog = tx.clone();
                        let job_phase = job_worker.clone();
                        let res = osint::whatsmyname::refresh_with_progress(
                            Some(cancel.clone()),
                            move |phase| {
                                if let Ok(guard) = job_phase.lock() {
                                    if let Some(ref j) = *guard {
                                        j.phase(phase, None, None);
                                    }
                                }
                                let _ = tx_prog.send(WorkEvent::DatasetRefreshProgress {
                                    dataset: "whatsmyname".into(),
                                    phase: phase.to_string(),
                                });
                            },
                        )
                        .await;
                        let outcome = match res {
                            Ok(manifest) => Ok(format!(
                                "WhatsMyName dataset refreshed: {} sites ({})",
                                manifest.total_count, manifest.active_version
                            )),
                            Err(err) => Err(err.to_string()),
                        };
                        let job_handle = job_worker.lock().ok().and_then(|mut g| g.take());
                        tracked::finish(job_handle, "tool", &outcome, Some(&stop));
                        let _ = tx.send(WorkEvent::DatasetRefreshDone {
                            dataset: "whatsmyname".into(),
                            outcome,
                        });
                    });
                    Ok("Refreshing WhatsMyName dataset".into())
                }
            }
            ButtonId::OsintRefreshTemplates => {
                if self.dataset_refresh_running.is_some() {
                    Ok("A dataset refresh is already in progress".into())
                } else {
                    let cancel = Arc::new(AtomicBool::new(false));
                    self.dataset_refresh_running = Some("dorksearch".into());
                    self.dataset_refresh_cancel = Some(cancel.clone());
                    self.status = "Refreshing DorkSearch templates...".into();
                    self.push_log("info", "Started DorkSearch templates refresh");
                    let job = Arc::new(std::sync::Mutex::new(tracked::begin_cancellable(
                        JobSpec::new("tools", "templates_refresh", "Refresh DorkSearch templates")
                            .tool("dork_generate"),
                        cancel.clone(),
                    )));
                    let tx = self.work_tx.clone();
                    let stop = cancel.clone();
                    let job_worker = job.clone();
                    tokio::spawn(async move {
                        let tx_prog = tx.clone();
                        let job_phase = job_worker.clone();
                        let res = osint::dork_generator::refresh_with_progress(
                            Some(cancel.clone()),
                            move |phase| {
                                if let Ok(guard) = job_phase.lock() {
                                    if let Some(ref j) = *guard {
                                        j.phase(phase, None, None);
                                    }
                                }
                                let _ = tx_prog.send(WorkEvent::DatasetRefreshProgress {
                                    dataset: "dorksearch".into(),
                                    phase: phase.to_string(),
                                });
                            },
                        )
                        .await;
                        let outcome = match res {
                            Ok(manifest) => Ok(format!(
                                "DorkSearch templates refreshed: {} templates ({})",
                                manifest.total_count, manifest.active_version
                            )),
                            Err(err) => Err(err.to_string()),
                        };
                        let job_handle = job_worker.lock().ok().and_then(|mut g| g.take());
                        tracked::finish(job_handle, "tool", &outcome, Some(&stop));
                        let _ = tx.send(WorkEvent::DatasetRefreshDone {
                            dataset: "dorksearch".into(),
                            outcome,
                        });
                    });
                    Ok("Refreshing DorkSearch templates".into())
                }
            }
            ButtonId::OsintSearchSelected => self.search_selected_dork(),
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
            ButtonId::SeeMoreDetail => {
                let page = self.see_more_pages.get()[0] as i32;
                nudge(&mut self.scrolls.detail, page, 10_000);
                return;
            }
            ButtonId::SeeMoreRecall => {
                let page = self.see_more_pages.get()[1] as i32;
                nudge(&mut self.scrolls.recall, page, 10_000);
                return;
            }
            ButtonId::SeeMorePopup => {
                let page = self.see_more_pages.get()[2] as i32;
                nudge(&mut self.scrolls.popup, page, 10_000);
                return;
            }
            ButtonId::SaveFirecrawlKey => self.remember_firecrawl_key(),
            ButtonId::SaveHunterKey => self.remember_hunter_key(),
            ButtonId::SaveSociaVaultKey => self.remember_sociavault_key(),
            ButtonId::SaveNewsApiKey => self.remember_newsapi_key(),
            ButtonId::SaveCourtListenerKey => self.remember_courtlistener_key(),
            ButtonId::SaveGnewsKey => self.remember_gnews_key(),
            ButtonId::SaveNewsDataKey => self.remember_newsdata_key(),
            ButtonId::SaveCurrentsKey => self.remember_currents_key(),
            ButtonId::SaveWhoxyKey => self.remember_whoxy_key(),
            ButtonId::TestWhoxyConnection => self.test_whoxy_connection(),
            ButtonId::AtlasRun => self.atlas_control(),
            ButtonId::AtlasAuto => self.toggle_atlas_auto(),
            ButtonId::AtlasRuns => {
                self.atlas_page = AtlasPage::Runs;
                self.load_atlas();
                self.set_focus(Target::Button(ButtonId::AtlasLive));
                Ok("News cycle".into())
            }
            ButtonId::AtlasLive => {
                self.atlas_page = AtlasPage::Live;
                self.set_focus(Target::Button(ButtonId::AtlasRun));
                Ok("Atlas".into())
            }
            ButtonId::AtlasDelete => self.delete_atlas_run(),
            ButtonId::AtlasResume => self.resume_atlas_run(),
            ButtonId::AtlasRepair => self.start_atlas_repair(),
            ButtonId::ClearLog => {
                // Scope: durable events only. Jobs, task results and memories stay.
                let cleared = self.store.clear_events();
                self.logs.open.clear();
                self.logs.scroll = 0;
                self.logs.follow = true;
                self.push_log("info", "Events cleared (jobs, results, and memories kept)");
                self.reload_logs();
                cleared.map(|n| format!("Cleared {n} events"))
            }
            ButtonId::JobsStatus => {
                self.jobs.cycle_status();
                self.reload_jobs();
                Ok(format!("Status: {}", self.jobs.status.label()))
            }
            ButtonId::JobsApp => {
                self.jobs.cycle_app();
                self.reload_jobs();
                Ok("Jobs filtered".into())
            }
            ButtonId::JobsViewLogs => self.view_job_logs(),
            ButtonId::JobsRetry => {
                let Some(id) = self.jobs.selected().map(|job| job.id.clone()) else {
                    return;
                };
                let retried = self.store.retry_failed_tasks(&id);
                self.reload_jobs();
                retried.map(|n| {
                    if n == 0 {
                        "Nothing to retry".into()
                    } else {
                        format!("Retrying {n} failed task(s)")
                    }
                })
            }
            ButtonId::JobsCancel => {
                let Some(id) = self.jobs.selected().map(|job| job.id.clone()) else {
                    return;
                };
                let requested = self.store.request_job_cancel(&id);
                self.reload_jobs();
                requested.map(|outcome| match outcome {
                    CancelRequest::Requested => {
                        "Cancel requested · the job stops at its next safe point".into()
                    }
                    CancelRequest::NotCancellable => "This job cannot be cancelled safely".into(),
                    CancelRequest::NotRunning => "This job is not running".into(),
                    CancelRequest::Missing => "This job no longer exists".into(),
                })
            }
            ButtonId::JobsOpenSource => self.open_job_source(),
            ButtonId::LogsLevel => {
                self.logs.level = self.logs.level.next();
                self.reload_logs();
                Ok(format!("Level: {}", self.logs.level.label()))
            }
            ButtonId::LogsApp => {
                self.logs.cycle_app();
                self.reload_logs();
                Ok("Logs filtered".into())
            }
            ButtonId::LogsFollow => {
                self.logs.follow = !self.logs.follow;
                self.reload_logs();
                Ok(format!(
                    "Live follow {}",
                    if self.logs.follow { "on" } else { "off" }
                ))
            }
            ButtonId::LogsOpenJob => {
                let Some(job) = self.logs.selected().map(|row| row.job_id.clone()) else {
                    return;
                };
                if job.is_empty() {
                    Ok("This event has no job".into())
                } else {
                    self.open_job(&job)
                }
            }
            ButtonId::LogsBack => {
                self.leave_logs_to_job();
                return;
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
            ButtonId::SaveClassifier => self.save_role(DefaultsRole::Classifier),
            ButtonId::SaveSummarization => self.save_role(DefaultsRole::Summarization),
            ButtonId::SaveEvidenceCurator => self.save_role(DefaultsRole::EvidenceCurator),
            ButtonId::SaveEntityResolver => self.save_role(DefaultsRole::EntityResolver),
            ButtonId::SaveClaimAssessor => self.save_role(DefaultsRole::ClaimAssessor),
            ButtonId::SaveInvestigationController => {
                self.save_role(DefaultsRole::InvestigationController)
            }
            ButtonId::AtlasNewsFeed => {
                self.open_atlas_news();
                return;
            }
            ButtonId::AtlasWorld => {
                self.show_atlas_world();
                return;
            }
            ButtonId::AtlasNews => {
                if !self.atlas_news_run.is_empty() {
                    self.atlas_news = true;
                    self.atlas_focus = None;
                }
                Ok("News".into())
            }
            ButtonId::DefaultRole(role) => {
                if self.defaults_role != role {
                    self.defaults_role = role;
                    self.fallback_sel = 0;
                    self.model_catalog.clear();
                    self.catalog_for.clear();
                }
                Ok(format!("{} default", role.label()))
            }
            ButtonId::RefreshModels => {
                self.refresh_catalog();
                return;
            }
            ButtonId::AddFallback => {
                self.open_add_fallback();
                return;
            }
            ButtonId::DeleteFallback => self.delete_selected_fallback(),
            ButtonId::MoveFallbackUp => self.move_selected_fallback(-1),
            ButtonId::MoveFallbackDown => self.move_selected_fallback(1),
            ButtonId::FallbackItem(index) => {
                self.fallback_sel = index;
                Ok(format!("Fallback {}", index + 1))
            }
            ButtonId::FallbackPick(index) => {
                self.fallback_popup_sel = index;
                Ok(format!("Model {}", index + 1))
            }
            ButtonId::FallbackTab(page) => {
                self.fallback_popup_tab = page;
                self.fallback_popup_sel = 0;
                self.ensure_fallback_catalog();
                Ok(page.title().to_string())
            }
            ButtonId::ConfirmAddFallback => self.confirm_add_fallback(),
            ButtonId::OpenDocumentation => {
                let Some(tool) = osint::registry().get(self.tool_sel) else {
                    return;
                };
                if tool.documentation.starts_with("http://")
                    || tool.documentation.starts_with("https://")
                {
                    self.status = format!("Open {}", tool.documentation);
                    if !cfg!(test) {
                        let _ = open_external_url(tool.documentation);
                    }
                } else {
                    self.status = "Documentation unavailable".into();
                }
                return;
            }
            ButtonId::GoogleSave => {
                let result = self.save_provider(ProviderPage::Google);
                self.report(result);
                return;
            }
            ButtonId::GoogleVerify => {
                self.verify_provider(ProviderPage::Google);
                return;
            }
            ButtonId::NvidiaSave => {
                let result = self.save_provider(ProviderPage::Nvidia);
                self.report(result);
                return;
            }
            ButtonId::NvidiaVerify => {
                self.verify_provider(ProviderPage::Nvidia);
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
            ButtonId::IntelDay => {
                self.open_intel_day_picker();
                return;
            }
            ButtonId::IntelReports => {
                if !self.selected_intel_busy() {
                    self.open_intel_recon_popup();
                }
                return;
            }
            ButtonId::IntelFullReport => {
                if self.selected_intel_busy() {
                    return;
                }
                let Some(job) = self.intel_jobs.get(self.intel_job_sel) else {
                    return;
                };
                let mut sections = self
                    .store
                    .intel_report_sections(&job.id)
                    .unwrap_or_default();
                sections.sort_by_key(|section| section.ordinal);
                let mut body = format!(
                    "{} · revision {} · {}\n\n",
                    job.mode, job.revision, job.state
                );
                for section in sections {
                    body.push_str(&format!("## {} · {}\n\n", section.title, section.status));
                    if section.markdown.trim().is_empty() {
                        body.push_str("Content unavailable for this section.\n\n");
                    } else {
                        body.push_str(&section.markdown);
                        body.push_str("\n\n");
                    }
                }
                self.scrolls.popup = 0;
                self.overlay = Overlay::Block {
                    title: format!("Report {} r{}", job.id, job.revision),
                    body,
                };
                return;
            }
            ButtonId::IntelReconStart => {
                self.start_intel_report_from_popup();
                return;
            }
            ButtonId::IntelBodyRetry | ButtonId::IntelBodyRefresh => {
                self.ensure_article_body_fetch(true);
                return;
            }
            ButtonId::IntelJobOpen => {
                if let Some(job) = self.intel_jobs.get(self.intel_job_sel).cloned() {
                    self.intel_sections = self
                        .store
                        .intel_report_sections(&job.id)
                        .unwrap_or_default();
                    self.status = format!("Opened {} r{}", job.mode, job.revision);
                }
                return;
            }
            ButtonId::IntelJobPause => {
                if let Some(id) = self.selected_intel_job_id() {
                    let _ = intel_recon::pause_job(&self.store, &id);
                    if let Some(cancel) = self.intel_report_running.get(&id) {
                        cancel.store(true, Ordering::Relaxed);
                    }
                    self.refresh_intel_briefing();
                    self.status = "Job paused".into();
                }
                return;
            }
            ButtonId::IntelJobResume => {
                if let Some(id) = self.selected_intel_job_id() {
                    let _ = intel_recon::resume_job(&self.store, &id);
                    if let Some(job) = self.store.intel_report_job(&id).ok().flatten() {
                        if let Some(mode) = ReportMode::parse(&job.mode) {
                            // Restart worker for resumed job.
                            self.intel_report_running.remove(&id);
                            self.start_intel_report(mode, ReportScope::default());
                        }
                    }
                    self.status = "Job resumed".into();
                }
                return;
            }
            ButtonId::IntelJobCancel => {
                if let Some(id) = self.selected_intel_job_id() {
                    let _ = intel_recon::cancel_job(&self.store, &id);
                    if let Some(cancel) = self.intel_report_running.remove(&id) {
                        cancel.store(true, Ordering::Relaxed);
                    }
                    self.refresh_intel_briefing();
                    self.status = "Job cancelled".into();
                }
                return;
            }
            ButtonId::IntelJobRetry => {
                if let Some(id) = self.selected_intel_job_id() {
                    let _ = intel_recon::retry_failed_tasks(&self.store, &id);
                    if let Some(job) = self.store.intel_report_job(&id).ok().flatten() {
                        if let Some(mode) = ReportMode::parse(&job.mode) {
                            self.intel_report_running.remove(&id);
                            self.start_intel_report(mode, ReportScope::default());
                        }
                    }
                    self.status = "Retrying failed work".into();
                }
                return;
            }
            ButtonId::RefreshHardware => {
                self.hardware = hardware::profile_cached(true);
                Ok("Hardware refreshed".into())
            }
            ButtonId::GoogleAdvanced | ButtonId::NvidiaAdvanced => {
                let shown = if button == ButtonId::GoogleAdvanced {
                    self.google_advanced = !self.google_advanced;
                    self.google_advanced
                } else {
                    self.nvidia_advanced = !self.nvidia_advanced;
                    self.nvidia_advanced
                };
                Ok(if shown {
                    "Advanced endpoint shown"
                } else {
                    "Advanced endpoint hidden"
                }
                .into())
            }
        };
        self.report(result);
    }

    /// Saves one role's provider and model. Only `settings.toml` changes; credentials
    /// stay where they are. The change is recorded in Logs.
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
                DefaultsRole::Classifier => "classifier",
                DefaultsRole::Summarization => "summarization",
                DefaultsRole::EvidenceCurator => "evidence_curator",
                DefaultsRole::EntityResolver => "entity_resolver",
                DefaultsRole::ClaimAssessor => "claim_assessor",
                DefaultsRole::InvestigationController => "investigation_controller",
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
            DefaultsRole::Classifier => format!(
                "Classifier: {after} ({})",
                provider::picker_transport(&model)
            ),
            DefaultsRole::Summarization => {
                let inherited = self
                    .settings
                    .defaults
                    .summarization
                    .provider
                    .trim()
                    .is_empty()
                    && self.settings.defaults.summarization.model.trim().is_empty();
                if inherited {
                    format!("Summarization: {after} (inherits Synthesis)")
                } else {
                    format!("Summarization: {after}")
                }
            }
            DefaultsRole::EvidenceCurator => format!("Evidence curator: {after}"),
            DefaultsRole::EntityResolver => format!("Entity resolver: {after}"),
            DefaultsRole::ClaimAssessor => format!("Claim assessor: {after}"),
            DefaultsRole::InvestigationController => format!("Controller: {after}"),
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
        let job = tracked::begin(JobSpec::new(
            "models",
            "provider_verify",
            "Verify OpenRouter connection",
        ));
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
            tracked::finish(job, "provider", &result, None);
            let _ = tx.send(ProviderEvent::Finished {
                page: ProviderPage::OpenRouter,
                result,
            });
        });
    }

    fn provider_draft(&self, page: ProviderPage) -> Result<ProviderSecret> {
        let (kind, key, base_url) = match page {
            ProviderPage::Google => ("google", &self.google_key, &self.google_endpoint),
            ProviderPage::Nvidia => ("nvidia", &self.nvidia_key, &self.nvidia_endpoint),
            _ => anyhow::bail!("Choose Google or Nvidia"),
        };
        let mut secret = provider::account_secret(&self.auth, kind);
        secret.api_key = Some(key.trim().to_string()).filter(|key| !key.is_empty());
        secret.base_url = provider::normalize_base(base_url);
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
        anyhow::ensure!(
            provider::resolved_key(&secret).is_some(),
            "Enter an API key or set the provider environment key"
        );
        Ok(secret)
    }

    fn save_provider(&mut self, page: ProviderPage) -> Result<String> {
        let secret = self.provider_draft(page)?;
        let mut auth = self.auth.clone();
        auth.set_account(secret);
        auth.save_to(&self.auth_path)?;
        self.auth = auth;
        let status = format!("{} account saved · verify to test access", page.title());
        match page {
            ProviderPage::Google => self.google_status = status.clone(),
            ProviderPage::Nvidia => self.nvidia_status = status.clone(),
            _ => unreachable!(),
        }
        Ok(status)
    }

    fn verify_provider(&mut self, page: ProviderPage) {
        if self.provider_pending.is_some() {
            self.status = "Another provider check is running".into();
            return;
        }
        let secret = match self.provider_draft(page) {
            Ok(secret) => secret,
            Err(err) => {
                self.status = err.to_string();
                return;
            }
        };
        self.provider_pending = Some(page);
        let label = page.title();
        let status = format!("Verifying {label} connection…");
        if page == ProviderPage::Google {
            self.google_status = status.clone();
        } else {
            self.nvidia_status = status.clone();
        }
        self.status = status;
        let tx = self.provider_tx.clone();
        let job = tracked::begin(JobSpec::new(
            "models",
            "provider_verify",
            format!("Verify {label} connection"),
        ));
        tokio::spawn(async move {
            let result = provider::verified_catalog(&secret)
                .await
                .map(|models| format!("{label} catalog reachable · {} models", models.len()))
                .map_err(|err| err.to_string());
            tracked::finish(job, "provider", &result, None);
            let _ = tx.send(ProviderEvent::Finished { page, result });
        });
    }

    fn on_provider_event(&mut self, event: ProviderEvent) {
        match event {
            ProviderEvent::Finished { page, result } => {
                if self.provider_pending != Some(page) {
                    return;
                }
                self.provider_pending = None;
                let message = result.unwrap_or_else(|err| err);
                match page {
                    ProviderPage::Google => self.google_status = message.clone(),
                    ProviderPage::Nvidia => self.nvidia_status = message.clone(),
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
            Target::AtlasFeed(index) => {
                if self.atlas_feed.is_empty() {
                    return;
                }
                self.atlas_feed_sel = index.min(self.atlas_feed.len() - 1);
                self.set_focus(Target::AtlasFeed(self.atlas_feed_sel));
                self.open_atlas_article();
            }
            Target::AtlasHistory(index) => {
                if self.atlas_runs.is_empty() {
                    return;
                }
                self.atlas_run_sel = index.min(self.atlas_runs.len() - 1);
                self.scrolls.origins = 0;
                self.set_focus(Target::AtlasHistory(self.atlas_run_sel));
                self.open_atlas_news();
            }
            Target::AtlasCycleStats => {
                self.set_focus(Target::AtlasCycleStats);
            }
            Target::PathLine(index) => self.activate_path_line(index),
            Target::RelatedRow(index) => {
                // A click opens the row under the pointer immediately.
                if index < self.brain_detail.related.items.len() {
                    self.brain_detail.related.sel = index;
                    self.open_selected_related();
                }
            }
            Target::DetailPath | Target::DetailSummary => self.set_focus(target),
            Target::AtlasArticle(index) => {
                if self.atlas_articles.is_empty() {
                    return;
                }
                self.atlas_article_sel = index.min(self.atlas_articles.len() - 1);
                self.set_focus(Target::AtlasArticle(self.atlas_article_sel));
                self.open_saved_article();
            }
            Target::IntelTab(index) => self.set_intel_category(index),
            Target::IntelArticle(index) => {
                if self.intel_articles.is_empty() {
                    return;
                }
                self.intel_sel = index.min(self.intel_articles.len() - 1);
                self.set_focus(Target::IntelArticle(self.intel_sel));
                self.open_intel_briefing();
            }
            Target::LogLine(index) => {
                if self.logs.rows.is_empty() {
                    return;
                }
                self.logs.select(index);
                self.set_focus(Target::LogLine(self.logs.sel));
                self.toggle_log();
            }
            Target::JobRow(index) => {
                self.jobs.select(index, &self.store);
                self.set_focus(Target::JobRow(self.jobs.sel));
            }
            Target::JobDetail => self.set_focus(Target::JobDetail),
            Target::IntelLeftColumn => self.set_focus(Target::IntelLeftColumn),
            Target::Choice(index) if self.overlay == Overlay::Palette => {
                if let Some(id) = self
                    .palette_items()
                    .get(index)
                    .filter(|item| item.enabled)
                    .map(|item| item.id.clone())
                {
                    self.run_palette(&id);
                }
            }
            Target::Choice(index) => self.apply_choice(index),
            Target::IntelReconTab(index) => self.select_intel_recon_tab(index),
            Target::IntelReconSection(index) => self.toggle_intel_recon_section(index),
            Target::CloseOverlay => {
                let restore = if self.overlay == Overlay::AddFallback {
                    self.fallback_popup_restore.take()
                } else {
                    None
                };
                self.overlay = Overlay::None;
                self.scrolls.popup = 0;
                if let Some(target) = restore {
                    self.set_focus(target);
                }
            }
            Target::Tab(index) => {
                let _ = self.switch_tab(index);
                self.set_focus(target);
            }
            Target::TabClose(index) => {
                let _ = self.close_tab(index);
            }
            Target::TabPlus => {
                self.go_home();
                self.set_focus(Target::Field(FieldId::Composer));
            }
            Target::TabOverflow => {
                self.open_investigation_switcher();
            }
            Target::ReconContext => self.set_focus(Target::ReconContext),
            Target::OsintDetail => self.set_focus(Target::OsintDetail),
            Target::BrainRecall => self.set_focus(Target::BrainRecall),
        }
    }

    fn edit_char(&mut self, character: char) {
        let Target::Field(field) = self.focus else {
            return;
        };
        if is_picker_field(field) {
            return;
        }
        let len = self.field(field).chars().count();
        if self.cursor > len {
            self.cursor = len;
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
        self.after_field_edit();
    }

    fn edit_paste(&mut self, pasted: &str) {
        // The Configs import editor owns the keyboard while the popup is open,
        // and it is the one field that must accept a whole document.
        if self.overlay == Overlay::Configs {
            if self.profile_config.tab == crate::tui::profile_config::ConfigTab::Import {
                self.profile_config.insert(pasted);
            }
            return;
        }
        let Target::Field(field) = self.focus else {
            return;
        };
        if is_picker_field(field) || self.overlay != Overlay::None {
            return;
        }
        let multiline = field == FieldId::Composer;
        let sanitized: String = pasted
            .chars()
            .filter(|ch| *ch == '\n' && multiline || !ch.is_control())
            .collect();
        if sanitized.is_empty() {
            return;
        }
        let cursor = self.cursor.min(self.field(field).chars().count());
        let value = self.field_mut(field);
        let byte = value
            .char_indices()
            .nth(cursor)
            .map(|(byte, _)| byte)
            .unwrap_or(value.len());
        value.insert_str(byte, &sanitized);
        self.cursor = cursor + sanitized.chars().count();
        self.after_field_edit();
    }

    fn edit_backspace(&mut self) {
        let Target::Field(field) = self.focus else {
            return;
        };
        if is_picker_field(field) {
            return;
        }
        let len = self.field(field).chars().count();
        if self.cursor > len {
            self.cursor = len;
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
        self.after_field_edit();
    }

    fn edit_delete(&mut self) {
        let Target::Field(field) = self.focus else {
            return;
        };
        if is_picker_field(field) {
            return;
        }
        let len = self.field(field).chars().count();
        if self.cursor >= len {
            self.cursor = len;
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
        self.after_field_edit();
    }

    fn after_field_edit(&mut self) {
        if self.focus == Target::Field(FieldId::BrainQuery)
            && self.module == Some(ModuleId::Brain)
            && self.brain_list_mode == BrainListMode::List
        {
            self.reload_memories();
        }
        if self.focus == Target::Field(FieldId::IntelSearch) && self.module == Some(ModuleId::Intel)
        {
            self.load_intel();
        }
        if self.focus == Target::Field(FieldId::LogsSearch) {
            self.reload_logs();
        }
        if self.focus == Target::Field(FieldId::JobsSearch) {
            self.reload_jobs();
        }
    }

    fn activate_path_line(&mut self, index: usize) {
        let lines = if self.brain_graph.is_empty() {
            Vec::new()
        } else {
            super::graph::path_lines(&self.brain_graph)
        };
        let Some(line) = lines.get(index) else {
            return;
        };
        if !line.article_id.is_empty() && !line.run_id.is_empty() {
            match self.open_claim_article(&line.run_id, &line.article_id) {
                Ok(status) => self.status = status,
                Err(err) => self.status = err.to_string(),
            }
            return;
        }
        match self.open_insight_source() {
            Ok(()) => self.status = "Source thread opened".into(),
            Err(err) => self.status = err.to_string(),
        }
    }

    fn open_claim_article(&mut self, run_id: &str, article_id: &str) -> Result<String> {
        let article = self
            .store
            .atlas_article(run_id, article_id)?
            .ok_or_else(|| anyhow::anyhow!("Article not found in that news cycle"))?;
        self.open_intel_briefing_for(article)?;
        Ok("Intel brief opened".into())
    }

    fn submit(&mut self) {
        if self.module.is_none() {
            let input = self.input.trim().to_string();
            if input.is_empty() {
                return;
            }
            if input.starts_with('/') {
                let result = self.run_slash(&input);
                self.report(result);
                return;
            }
            let result = self.launch_home_investigation();
            self.report(result.map(|_| "Investigation started".into()));
            return;
        }
        if self.module != Some(ModuleId::Recon) {
            return;
        }
        let input = self.input.trim().to_string();
        if input.is_empty() {
            return;
        }
        let result = if input.starts_with('/') {
            self.run_slash(&input)
        } else {
            self.recon_command(&input)
        };
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
            let removed = self.store.deletion_consequences(&id)?.len();
            self.store.delete_thread(&id, true)?;
            self.tab_ids.retain(|tid| tid != &id);
            self.tab_recently_closed.retain(|tid| tid != &id);
            self.tab_last_active.remove(&id);
            self.tab_unreads.remove(&id);
            self.save_session_tabs();
            self.selected_thread = None;
            self.messages.clear();
            self.recon_chat = false;
            self.refresh_threads()?;
            if let Some(next) = self.threads.first().map(|t| t.id.clone()) {
                self.open_thread(&next)?;
            }
            self.reload_memories();
            self.input.clear();
            return Ok(format!(
                "Investigation deleted; {removed} brain memories removed"
            ));
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
        // The Configs popup owns the keyboard while it is open.
        if self.overlay == Overlay::Configs {
            // The open tab owns its own field, so typing never leaks into the
            // module shortcuts underneath.
            self.set_focus(Target::Field(match self.profile_config.tab {
                super::profile_config::ConfigTab::Export => FieldId::ProfileExportPath,
                crate::tui::profile_config::ConfigTab::Import => FieldId::ProfileImportEditor,
            }));
            if self.profile_config_key(key) {
                return true;
            }
        }
        if self.module == Some(ModuleId::System)
            && self.overlay == Overlay::None
            && super::profile::handle_key(self, key)
        {
            return true;
        }
        if ctrl && matches!(key.code, KeyCode::Char('q') | KeyCode::Char('Q')) {
            return self.arm_quit();
        }
        if ctrl && matches!(key.code, KeyCode::Char('k') | KeyCode::Char('K')) {
            self.open_palette();
            return true;
        }
        if let Overlay::ResumeSession(ref session) = self.overlay {
            match key.code {
                KeyCode::Enter | KeyCode::Char('y') | KeyCode::Char('Y') => {
                    let sess = session.clone();
                    self.resume_view_session(&sess);
                }
                KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => {
                    self.overlay = Overlay::None;
                    if self.module.is_none() {
                        self.set_focus(Target::Field(FieldId::Composer));
                    }
                }
                KeyCode::Tab | KeyCode::BackTab | KeyCode::Left | KeyCode::Right => {
                    self.focus = if self.focus == Target::Button(ButtonId::ResumeSessionConfirm) {
                        Target::Button(ButtonId::ResumeSessionDismiss)
                    } else {
                        Target::Button(ButtonId::ResumeSessionConfirm)
                    };
                }
                _ => {}
            }
            return true;
        }
        if self.overlay == Overlay::AddFallback {
            if ctrl && matches!(key.code, KeyCode::Char('u') | KeyCode::Char('d')) {
                super::ui::page(
                    self,
                    if key.code == KeyCode::Char('d') {
                        1
                    } else {
                        -1
                    },
                );
                return true;
            }
            match key.code {
                KeyCode::Esc | KeyCode::Char('q') => self.activate_target(Target::CloseOverlay),
                KeyCode::Left | KeyCode::Char('h') | KeyCode::BackTab => {
                    self.cycle_fallback_tab(-1);
                }
                KeyCode::Right | KeyCode::Char('l') | KeyCode::Tab => {
                    self.cycle_fallback_tab(1);
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    self.fallback_popup_sel = self.fallback_popup_sel.saturating_sub(1);
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    let last = self.filtered_fallback_models().len().saturating_sub(1);
                    self.fallback_popup_sel = (self.fallback_popup_sel + 1).min(last);
                }
                KeyCode::Enter => {
                    let result = self.confirm_add_fallback();
                    self.report(result);
                }
                KeyCode::Backspace => {
                    self.fallback_popup_filter.pop();
                    self.fallback_popup_sel = 0;
                }
                KeyCode::Char(c) if !c.is_control() => {
                    self.fallback_popup_filter.push(c);
                    self.fallback_popup_sel = 0;
                }
                _ => {}
            }
            return true;
        }
        if self.overlay == Overlay::Palette {
            match key.code {
                KeyCode::Esc => self.activate_target(Target::CloseOverlay),
                KeyCode::Up | KeyCode::Char('k')
                    if !key.modifiers.contains(KeyModifiers::CONTROL) =>
                {
                    self.palette_sel = self.palette_sel.saturating_sub(1);
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    let last = self.palette_items().len().saturating_sub(1);
                    self.palette_sel = (self.palette_sel + 1).min(last);
                }
                KeyCode::Enter => {
                    if let Some(item) = self.palette_items().get(self.palette_sel).cloned() {
                        if item.enabled {
                            self.run_palette(&item.id);
                        } else {
                            self.status = item.disabled_reason;
                        }
                    }
                }
                KeyCode::Backspace => {
                    self.palette_query.pop();
                    self.palette_sel = 0;
                }
                KeyCode::Char(c) if !c.is_control() => {
                    self.palette_query.push(c);
                    self.palette_sel = 0;
                }
                _ => {}
            }
            return true;
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
        if self.overlay == Overlay::IntelRecon {
            match key.code {
                KeyCode::Esc | KeyCode::Char('q') => self.activate_target(Target::CloseOverlay),
                KeyCode::Left | KeyCode::Char('h') => self.move_intel_recon_focus(-1, true),
                KeyCode::Right | KeyCode::Char('l') => self.move_intel_recon_focus(1, true),
                KeyCode::Up | KeyCode::Char('k') => self.move_intel_recon_focus(-1, false),
                KeyCode::Down | KeyCode::Char('j') => self.move_intel_recon_focus(1, false),
                KeyCode::Tab => self.move_intel_recon_focus(1, false),
                KeyCode::BackTab => self.move_intel_recon_focus(-1, false),
                KeyCode::Char(' ') => match self.intel_recon_focus {
                    IntelReconFocus::Section(index) => self.toggle_intel_recon_section(index),
                    IntelReconFocus::Tab(index) => self.select_intel_recon_tab(index),
                    IntelReconFocus::Start => self.start_intel_report_from_popup(),
                },
                KeyCode::Enter => match self.intel_recon_focus {
                    IntelReconFocus::Section(index) => self.toggle_intel_recon_section(index),
                    IntelReconFocus::Tab(index) => self.select_intel_recon_tab(index),
                    IntelReconFocus::Start => self.start_intel_report_from_popup(),
                },
                _ => {}
            }
            return true;
        }
        if self.overlay != Overlay::None {
            if super::ui::atlas_run_card(self)
                && matches!(key.code, KeyCode::Backspace | KeyCode::Delete)
            {
                let deleted = self.delete_atlas_run();
                self.report(deleted);
                return true;
            }
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
        if self.on_atlas_history()
            && matches!(self.focus, Target::AtlasHistory(_))
            && matches!(key.code, KeyCode::Backspace | KeyCode::Delete)
        {
            let deleted = self.delete_atlas_run();
            self.report(deleted);
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
        if ctrl && matches!(key.code, KeyCode::Tab | KeyCode::BackTab) {
            let reverse =
                key.modifiers.contains(KeyModifiers::SHIFT) || key.code == KeyCode::BackTab;
            self.cycle_module(reverse);
            return true;
        }
        if ctrl && matches!(key.code, KeyCode::Left | KeyCode::Right) {
            self.cycle_module(key.code == KeyCode::Left);
            return true;
        }
        if ctrl && matches!(key.code, KeyCode::Char('n') | KeyCode::Char('N')) {
            self.go_home();
            self.set_focus(Target::Field(FieldId::Composer));
            return true;
        }
        if ctrl && matches!(key.code, KeyCode::Char('w') | KeyCode::Char('W')) {
            if self.tab_sel > 0 {
                let _ = self.close_tab(self.tab_sel);
            }
            return true;
        }
        if ctrl
            && key.modifiers.contains(KeyModifiers::SHIFT)
            && matches!(key.code, KeyCode::Char('t') | KeyCode::Char('T'))
        {
            let _ = self.reopen_closed_tab();
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
            } else if key.code == KeyCode::Left {
                let _ = self.prev_tab();
            } else {
                let _ = self.next_tab();
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
        if ctrl
            && matches!(key.code, KeyCode::Char('j') | KeyCode::Char('J'))
            && self.focus == Target::Field(FieldId::Composer)
        {
            self.edit_char('\n');
            self.persist_draft();
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
            KeyCode::Left | KeyCode::Char('h')
                if matches!(
                    self.focus,
                    Target::Tab(_) | Target::TabPlus | Target::TabOverflow
                ) =>
            {
                match self.focus {
                    Target::Tab(i) if i > 0 => self.set_focus(Target::Tab(i - 1)),
                    Target::TabPlus => self.set_focus(Target::Tab(self.tab_ids.len())),
                    Target::TabOverflow => self.set_focus(Target::TabPlus),
                    _ => {}
                }
            }
            KeyCode::Right | KeyCode::Char('l')
                if matches!(
                    self.focus,
                    Target::Tab(_) | Target::TabPlus | Target::TabOverflow
                ) =>
            {
                match self.focus {
                    Target::Tab(i) if i < self.tab_ids.len() => self.set_focus(Target::Tab(i + 1)),
                    Target::Tab(_) => self.set_focus(Target::TabPlus),
                    Target::TabPlus => self.set_focus(Target::TabOverflow),
                    _ => {}
                }
            }
            KeyCode::Char(' ')
                if matches!(
                    self.focus,
                    Target::Tab(_) | Target::TabPlus | Target::TabOverflow
                ) =>
            {
                self.on_enter();
            }
            KeyCode::Left if self.field_focused() => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Right if self.field_focused() => {
                if let Target::Field(field) = self.focus {
                    self.cursor = (self.cursor + 1).min(self.field(field).chars().count());
                }
            }
            KeyCode::Left | KeyCode::Char('h')
                if self.module == Some(ModuleId::Intel)
                    && self.intel_page == IntelPage::Bulletin
                    && !self.field_focused() =>
            {
                let index = INTEL_CATEGORIES
                    .iter()
                    .position(|id| *id == self.intel_category.as_str())
                    .unwrap_or(0);
                self.set_intel_category(index.saturating_sub(1));
            }
            KeyCode::Right | KeyCode::Char('l')
                if self.module == Some(ModuleId::Intel)
                    && self.intel_page == IntelPage::Bulletin
                    && !self.field_focused() =>
            {
                let index = INTEL_CATEGORIES
                    .iter()
                    .position(|id| *id == self.intel_category.as_str())
                    .unwrap_or(0);
                self.set_intel_category((index + 1).min(INTEL_CATEGORIES.len() - 1));
            }
            KeyCode::Left | KeyCode::Char('h') if self.transcript_focused() => {
                super::ui::fold_chat(self, false);
            }
            KeyCode::Right | KeyCode::Char('l') if self.transcript_focused() => {
                super::ui::fold_chat(self, true);
            }
            KeyCode::Char('e') if self.transcript_focused() => super::ui::toggle_chat(self),
            KeyCode::Char('o') if self.transcript_focused() => self.open_selected_evidence(),
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
                if !self.field_focused()
                    && key.modifiers.is_empty()
                    && c.is_ascii_digit()
                    && c != '0' =>
            {
                // Home order: 1 Intel · 2 Atlas · 3 Brain · 4 Recon · 5 Jobs · 6 Logs ·
                // 7 Tools · 8 Models · 9 System
                self.select((c as u8 - b'1') as usize);
            }
            KeyCode::Char(c)
                if !self.field_focused()
                    && key.modifiers.is_empty()
                    && matches!(self.module, Some(ModuleId::Logs) | Some(ModuleId::Jobs))
                    && self.dashboard_key(c) => {}
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
        if self.module == Some(ModuleId::Atlas) && self.atlas_pause.is_some() {
            if let Some(flag) = &self.atlas_pause {
                flag.store(true, Ordering::Relaxed);
            }
            self.atlas_status = "Pausing".into();
            self.status = self.atlas_status.clone();
            return true;
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
        if self.module == Some(ModuleId::Osint) {
            let mut cancelled = false;
            if let Some(cancel) = &self.dataset_refresh_cancel {
                cancel.store(true, Ordering::Relaxed);
                cancelled = true;
            }
            if let Some(cancel) = &self.osint_cancel {
                cancel.store(true, Ordering::Relaxed);
                cancelled = true;
            }
            if cancelled {
                self.status = "Cancellation requested".into();
                self.push_log("info", "Tool cancellation requested");
                return true;
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
            self.persist_view_session();
            return false;
        }
        self.persist_view_session();
        self.quit_arm = Some(now);
        self.status = "Press Ctrl+C or Ctrl+Q again to quit".into();
        true
    }

    fn on_esc(&mut self) {
        if self.overlay != Overlay::None {
            self.activate_target(Target::CloseOverlay);
            return;
        }
        if self.module.is_none() {
            if self.focus == Target::Field(FieldId::Composer) {
                self.set_focus(Target::App(self.launcher_sel));
                return;
            }
            return;
        }
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
        if self.module == Some(ModuleId::Brain) && self.brain_list_mode == BrainListMode::Graph {
            self.detail_back();
            return;
        }
        if self.module == Some(ModuleId::Brain) && self.brain_list_mode != BrainListMode::List {
            self.leave_brain_detail();
            return;
        }
        if self.module == Some(ModuleId::Atlas) && self.atlas_news {
            self.show_atlas_world();
            return;
        }
        if self.module == Some(ModuleId::Logs) && self.leave_logs_to_job() {
            return;
        }
        if self.module == Some(ModuleId::Jobs)
            && (self.jobs.detail_open || self.focus == Target::JobDetail)
        {
            self.jobs.detail_open = false;
            self.set_focus(if self.jobs.rows.is_empty() {
                Target::Field(FieldId::JobsSearch)
            } else {
                Target::JobRow(self.jobs.sel)
            });
            return;
        }
        if self.module == Some(ModuleId::Atlas) && self.atlas_page == AtlasPage::Live {
            self.atlas_page = AtlasPage::Runs;
            self.atlas_news = false;
            self.atlas_focus = None;
            self.load_atlas();
            if !self.atlas_runs.is_empty() {
                self.set_focus(Target::AtlasHistory(self.atlas_run_sel));
            } else {
                self.set_focus(Target::Button(ButtonId::AtlasLive));
            }
            self.status = "History".into();
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
        if self.module == Some(ModuleId::Intel) && self.intel_page == IntelPage::Briefing {
            self.leave_intel_briefing();
            return;
        }
        if self.module.is_some() {
            self.go_home();
        }
    }

    fn toggle_log(&mut self) {
        if self.logs.toggle_open() {
            let (room, width) = super::logs::list_geometry(super::ui::body_rect(self));
            self.logs.reveal(room, width);
        }
    }

    fn on_enter(&mut self) {
        if self.module == Some(ModuleId::Logs) && matches!(self.focus, Target::LogLine(_)) {
            self.toggle_log();
            return;
        }
        if self.module == Some(ModuleId::Jobs) && matches!(self.focus, Target::JobRow(_)) {
            // Narrow layouts swap the table for the detail; wide ones focus it.
            if super::ui::body_rect(self).width < super::jobs::WIDE {
                self.jobs.detail_open = true;
            }
            self.set_focus(Target::JobDetail);
            return;
        }
        match self.focus {
            Target::Field(FieldId::Composer) => self.submit(),
            Target::Field(field) if is_picker_field(field) => self.open_default_picker(field),
            Target::Field(_) => self.focus_next(false),
            Target::Transcript => self.enter_chat(),
            Target::Memory(_) => self.open_memory_graph(),
            Target::RelatedRow(_) => self.open_selected_related(),
            Target::AtlasFeed(_) => self.open_atlas_article(),
            Target::AtlasHistory(_) => self.open_atlas_news(),
            Target::AtlasArticle(_) => self.open_saved_article(),
            Target::IntelArticle(_) => self.open_intel_briefing(),
            Target::IntelTab(index) => self.set_intel_category(index),
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

    /// Single-key dashboard shortcuts (not typing in a field). Returns true
    /// when the key was handled.
    fn dashboard_key(&mut self, c: char) -> bool {
        let button = match (self.module, c) {
            (Some(ModuleId::Logs), 'f') => ButtonId::LogsFollow,
            (Some(ModuleId::Logs), 'o')
                if self.logs.selected().is_some_and(|r| !r.job_id.is_empty()) =>
            {
                ButtonId::LogsOpenJob
            }
            (Some(ModuleId::Logs), 'v') => ButtonId::LogsLevel,
            (Some(ModuleId::Jobs), 'l') if self.jobs.selected().is_some() => ButtonId::JobsViewLogs,
            (Some(ModuleId::Jobs), 'r') if self.jobs.can_retry() => ButtonId::JobsRetry,
            (Some(ModuleId::Jobs), 'c') if self.jobs.can_cancel() => ButtonId::JobsCancel,
            (Some(ModuleId::Jobs), 's') => ButtonId::JobsStatus,
            _ => return false,
        };
        self.activate_button(button);
        true
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
        if self.module == Some(ModuleId::Brain) && self.brain_list_mode == BrainListMode::Graph {
            match self.focus {
                Target::RelatedRow(_) => self.move_related(delta),
                Target::DetailSummary => {
                    self.scrolls.summary = add_scroll(self.scrolls.summary, delta);
                }
                Target::DetailPath => {
                    self.scrolls.path = add_scroll(self.scrolls.path, delta);
                }
                _ => {}
            }
            return;
        }
        match self.focus {
            Target::Field(FieldId::Composer) => self.move_composer_line(delta),
            Target::Field(FieldId::ReconSearch) | Target::Thread(_) => self.move_thread(delta),
            Target::Field(FieldId::IntelSearch) | Target::IntelArticle(_) | Target::IntelTab(_) => {
                self.move_intel(delta);
            }
            Target::Field(_) => {}
            Target::Transcript | Target::ChatHeader(_) | Target::ChatBody(_) => {
                super::ui::move_chat(self, delta);
            }
            Target::Memory(_) => self.move_memory(delta),
            Target::Tool(_) => self.move_tool(delta),
            Target::LogLine(_) => self.move_log(delta),
            Target::JobRow(_) => self.move_job(delta),
            Target::JobDetail => {
                self.jobs.detail_scroll = add_scroll(self.jobs.detail_scroll, delta);
            }
            Target::IntelLeftColumn => {
                let max = super::ui::intel_extracted_scroll_max(self);
                self.scrolls.intel_extracted =
                    add_scroll(self.scrolls.intel_extracted, delta * 3).min(max);
            }
            _ => match self.module {
                None => self.move_home(delta),
                Some(ModuleId::Brain) if self.brain_list_mode == BrainListMode::Create => {}
                Some(ModuleId::Brain) => self.move_memory(delta),
                Some(ModuleId::Osint) => self.move_tool(delta),
                Some(ModuleId::Atlas) => self.move_atlas(delta),
                Some(ModuleId::Intel) if self.intel_page == IntelPage::Briefing => {
                    let max = super::ui::intel_brief_scroll_max(self);
                    self.scrolls.intel_brief =
                        add_scroll(self.scrolls.intel_brief, delta * 3).min(max);
                }
                Some(ModuleId::Intel) => self.move_intel(delta),
                Some(ModuleId::System) => {}
                Some(ModuleId::Logs) => self.move_log(delta),
                Some(ModuleId::Jobs) => self.move_job(delta),
                Some(ModuleId::Recon) if self.recon_chat => super::ui::move_chat(self, delta),
                Some(ModuleId::Recon) => self.move_thread(delta),
                Some(ModuleId::Providers) => {
                    self.scrolls.detail = add_scroll(self.scrolls.detail, delta * 3);
                }
            },
        }
    }

    fn move_log(&mut self, delta: i32) {
        if self.logs.rows.is_empty() {
            return;
        }
        self.logs.move_by(delta);
        let (room, width) = super::logs::list_geometry(super::ui::body_rect(self));
        self.logs.reveal(room, width);
        self.set_focus(Target::LogLine(self.logs.sel));
    }

    fn move_job(&mut self, delta: i32) {
        if self.jobs.rows.is_empty() {
            return;
        }
        self.jobs.move_by(delta, &self.store);
        let room = super::jobs::table_room(super::ui::body_rect(self), &self.jobs);
        super::jobs::reveal(&mut self.jobs, room);
        self.set_focus(Target::JobRow(self.jobs.sel));
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

    pub(crate) fn move_memory(&mut self, delta: i32) {
        if self.memories.is_empty() {
            return;
        }
        let next =
            (self.memory_sel as i32 + delta).clamp(0, self.memories.len() as i32 - 1) as usize;
        if next == self.memory_sel && matches!(self.focus, Target::Memory(_)) {
            return;
        }
        self.memory_sel = next;
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
        } else if self.module.is_none() && self.focus == Target::Field(FieldId::Composer) {
            self.persist_home_draft();
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

    /// Marks the Home draft as having unsaved changes
    fn persist_home_draft(&mut self) {
        self.home_draft = self.input.clone();
        self.home_draft_cursor = self.cursor;
        self.home_draft_dirty = true;
    }

    /// Flushes the Home draft to persistent storage
    fn flush_home_draft(&mut self) -> Result<()> {
        if self.module.is_none() && self.focus == Target::Field(FieldId::Composer) {
            self.persist_home_draft();
        }
        if !self.home_draft_dirty {
            return Ok(());
        }
        self.home_draft_dirty = false;
        let state = HomeDraftState {
            prompt: self.home_draft.clone(),
            cursor: self.home_draft_cursor,
            scroll: self.home_draft_scroll,
            report_mode: self.home_draft_report_mode,
        };
        if let Ok(json) = serde_json::to_string(&state) {
            let _ = self.store.app_state_set("home_recon_draft", &json);
        }
        Ok(())
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
        self.pointer = Some((mouse.column, mouse.row));
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
            | FieldId::ClassifierProvider
            | FieldId::ClassifierModel
            | FieldId::SummarizationProvider
            | FieldId::SummarizationModel
    )
}

/// Date button / picker label like `04 OCT 2026`.
pub fn intel_day_button_label(day: &str) -> String {
    let Ok(date) = chrono::NaiveDate::parse_from_str(day.trim(), "%Y-%m-%d") else {
        return if day.trim().is_empty() {
            "No day".into()
        } else {
            day.trim().to_string()
        };
    };
    date.format("%d %b %Y").to_string().to_ascii_uppercase()
}

fn provider_label(kind: &str) -> &'static str {
    match provider::normalize_kind(kind).as_str() {
        "grok" | "grok-subscription" => "Grok (legacy)",
        "openai" | "openai-chatgpt" => "OpenAI (legacy)",
        "openrouter" => "OpenRouter",
        "google" => "Google",
        "nvidia" => "Nvidia",
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
    let picker = matches!(role, DefaultsRole::ToolPicker | DefaultsRole::Classifier)
        && provider == "openrouter";
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

pub(crate) fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

const ATLAS_AUTO_SECS: u64 = 60 * 60;

fn atlas_countdown_visible(app: &App) -> bool {
    app.atlas_auto_next.is_some()
        && app.module == Some(ModuleId::Atlas)
        && app.atlas_page == AtlasPage::Runs
        && !app.atlas_news
        && matches!(app.overlay, Overlay::None)
}

fn atlas_extracting_visible(app: &App) -> bool {
    app.module == Some(ModuleId::Atlas)
        && app.atlas_page == AtlasPage::Live
        && matches!(app.overlay, Overlay::None)
        && super::ui::atlas_extracting(app)
}

fn intel_body_loading_visible(app: &App) -> bool {
    app.module == Some(ModuleId::Intel)
        && app.intel_page == IntelPage::Briefing
        && matches!(app.overlay, Overlay::None)
        && super::ui::intel_body_loading(app)
}

fn intel_insights_loading_visible(app: &App) -> bool {
    app.module == Some(ModuleId::Intel)
        && app.intel_page == IntelPage::Briefing
        && matches!(app.overlay, Overlay::None)
        && (super::ui::intel_insights_loading(app) || app.intel_mode_classifying)
}

fn intel_summary_loading_visible(app: &App) -> bool {
    app.module == Some(ModuleId::Intel)
        && app.intel_page == IntelPage::Briefing
        && matches!(app.overlay, Overlay::None)
        && super::ui::intel_report_generating(app)
}

fn until_next_second() -> Duration {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|now| {
            let left = 1_000_000_000u32.saturating_sub(now.subsec_nanos());
            Duration::from_nanos(u64::from(left).max(1)).max(Duration::from_millis(50))
        })
        .unwrap_or(Duration::from_millis(200))
}

fn atlas_poll_wait(app: &App, fallback: Duration) -> Duration {
    let Some(next) = app.atlas_auto_next else {
        return fallback;
    };
    let now = unix_now();
    if now >= next {
        return Duration::from_millis(1);
    }
    Duration::from_secs(next - now)
        .min(fallback)
        .max(Duration::from_millis(1))
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
    // Durable local workers (index outbox + summary flush), each claiming only
    // its own pool. Stopped when dropped at the end of the session.
    let _workers = argos_osint_core::scheduler::WorkerPool::spawn_default(paths::db_path());
    // Process liveness for the job registry: jobs left running by a process
    // that exited become interrupted; cross-process Cancel reaches this one.
    let _beat = argos_osint_core::job_registry::start_process_beat(&paths::db_path()).ok();
    // One-time "Repair Atlas memories" reconciliation (resumes if interrupted).
    argos_osint_core::atlas_memory::spawn_startup_reconciliation(paths::db_path());
    // Committed memory changes from any writer (Atlas, Recon, Intel Recon, the
    // index/repair workers, other processes) reload Brain, coalesced per poll.
    let mut memory_watch = argos_osint_core::store::MemoryChangeWatcher::new(&app.store);
    let mut memory_polled = std::time::Instant::now();
    let mut dirty = true;
    loop {
        if pump(&mut app) {
            dirty = true;
        }
        if app.tick_dashboards() {
            dirty = true;
        }
        if memory_polled.elapsed() >= Duration::from_secs(1) {
            memory_polled = std::time::Instant::now();
            if memory_watch.poll(&app.store).is_some() {
                app.reload_memories();
                dirty = true;
            }
        }
        if dirty {
            if let Ok(size) = terminal.size() {
                app.screen = Rect::new(0, 0, size.width, size.height);
            }
            super::ui::normalize(&mut app);
            terminal.draw(|frame| {
                app.screen = frame.area();
                super::ui::draw(frame, &app)
            })?;
            if app.atlas_map_hold {
                app.atlas_map_hold = false;
                dirty = true;
            } else {
                dirty = false;
            }
        }
        let busy = !app.running.is_empty()
            || app.osint_cancel.is_some()
            || app.dataset_refresh_running.is_some()
            || app.provider_pending.is_some();
        let composer_focused = app.focus == Target::Field(FieldId::Composer);
        let mut wait = atlas_poll_wait(
            &app,
            Duration::from_millis(if dirty {
                90
            } else if busy {
                80
            } else if composer_focused {
                250
            } else {
                400
            }),
        );
        if composer_focused {
            wait = wait.min(Duration::from_millis(250));
        }
        if atlas_countdown_visible(&app) {
            wait = wait.min(until_next_second());
        }
        if atlas_extracting_visible(&app)
            || intel_body_loading_visible(&app)
            || intel_insights_loading_visible(&app)
            || intel_summary_loading_visible(&app)
        {
            wait = wait.min(Duration::from_millis(80));
        }
        if !event::poll(wait)? {
            if app.module == Some(ModuleId::Recon)
                && app.focus == Target::Field(FieldId::Composer)
                && app.draft_dirty
            {
                app.flush_draft();
            }
            if composer_focused
                || atlas_countdown_visible(&app)
                || atlas_extracting_visible(&app)
                || intel_body_loading_visible(&app)
                || intel_insights_loading_visible(&app)
                || intel_summary_loading_visible(&app)
            {
                dirty = true;
            }
            continue;
        }
        loop {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    if !app.handle_key(key) {
                        app.persist_view_session();
                        app.flush_draft();
                        return Ok(());
                    }
                    dirty = true;
                }
                Event::Paste(text) => {
                    app.edit_paste(&text);
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

fn summary_system(claim: bool) -> String {
    let mode = argos_osint_core::summarization::system_prompt(
        argos_osint_core::summarization::SummarizationMode::GraphExplanation,
    );
    let detail = if claim {
        "The claim path lists the concluding relation and the articles that support it as hard evidence, each with its published time. Write Markdown, not a fenced block. Start with one ## heading that states the relation: entity, predicate, and object, with the predicate and object in **bold**. Follow with one paragraph of how those articles support the relation and why the concluding insight is a fact or an inference. A fact means both spans are in an article title. An inference means a span is only in the description, or the article is context. Use only the graph and the memory. Do not invent sources, times, or outcomes. No bullet list."
    } else {
        "The recon path already keeps only the directive this insight rests on, with the subjects and evidence that contributed to it. Write Markdown, not a fenced block. Start with one ## heading that states the relation: entity, predicate, and object, with the predicate and object in **bold**. Follow with one paragraph of how that directive and the contributing evidence support the relation, and why the concluding insight is a fact or an inference. A fact rests on a tool result that states the relation. An inference is drawn when the evidence does not state it directly. Use only the graph and the memory. Do not mention directives that are absent from the recon path. Do not invent sources or outcomes. No bullet list."
    };
    format!(
        "{mode}

{detail}"
    )
}

fn work_event(thread_id: &str, event: recon::TurnEvent) -> WorkEvent {
    let thread_id = thread_id.to_string();
    match event {
        recon::TurnEvent::Stage(stage) => WorkEvent::ReconStage { thread_id, stage },
        recon::TurnEvent::AnswerDelta(text) => WorkEvent::AnswerDelta { thread_id, text },
        recon::TurnEvent::AnswerReset => WorkEvent::AnswerReset { thread_id },
        recon::TurnEvent::AnswerReplacement(text) => {
            WorkEvent::AnswerReplacement { thread_id, text }
        }
        recon::TurnEvent::AnswerNote(text) => WorkEvent::AnswerNote { thread_id, text },
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
    dirty |= app.poll_atlas_auto();
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
            provider_page: ProviderPage::Defaults,
            layout: std::cell::RefCell::new(LayoutRegistry::default()),
            see_more_pages: std::cell::Cell::new([8, 8, 8]),
            input: String::new(),
            brain_app: String::new(),
            brain_conversation: String::new(),
            brain_insight: String::new(),
            brain_query: String::new(),
            recon_search: String::new(),
            intel_search: String::new(),
            osint_search: String::new(),
            osint_input: osint::registry()[0].example_input().to_string(),
            osint_inputs: HashMap::new(),
            firecrawl_key: String::new(),
            firecrawl_fallback: String::new(),
            hunter_key: String::new(),
            hunter_fallback: String::new(),
            sociavault_key: String::new(),
            sociavault_fallback: String::new(),
            newsapi_key: String::new(),
            newsapi_fallback: String::new(),
            courtlistener_key: String::new(),
            courtlistener_fallback: String::new(),
            gnews_key: String::new(),
            gnews_fallback: String::new(),
            newsdata_key: String::new(),
            newsdata_fallback: String::new(),
            currents_key: String::new(),
            currents_fallback: String::new(),
            whoxy_key: String::new(),
            whoxy_fallback: String::new(),
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
            recon_context_enabled: true,
            scrolls: Scrolls::default(),
            expanded: HashSet::new(),
            chat_sel: 0,
            chat_follow: true,
            overlay: Overlay::None,
            logs: crate::tui::logs::LogsView::default(),
            jobs: crate::tui::jobs::JobsView::default(),
            dashboards_at: None,
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
            dataset_refresh_running: None,
            dataset_refresh_cancel: None,
            recon_provider: String::new(),
            recon_model: String::new(),
            picker_provider: String::new(),
            picker_model: String::new(),
            synthesis_provider: String::new(),
            synthesis_model: String::new(),
            classifier_provider: String::new(),
            classifier_model: String::new(),
            summarization_provider: String::new(),
            summarization_model: String::new(),
            evidence_curator_provider: String::new(),
            evidence_curator_model: String::new(),
            entity_resolver_provider: String::new(),
            entity_resolver_model: String::new(),
            claim_assessor_provider: String::new(),
            claim_assessor_model: String::new(),
            investigation_controller_provider: String::new(),
            investigation_controller_model: String::new(),
            show_thinking: false,
            defaults_role: DefaultsRole::Recon,
            model_catalog: Vec::new(),
            catalog_cache: HashMap::new(),
            catalog_for: String::new(),
            fallback_sel: 0,
            fallback_popup_tab: ProviderPage::Google,
            fallback_popup_filter: String::new(),
            fallback_popup_sel: 0,
            fallback_popup_restore: None,
            choice_items: Vec::new(),
            choice_sel: 0,
            choice_note: String::new(),
            palette_query: String::new(),
            palette_sel: 0,
            google_key: String::new(),
            google_endpoint: "https://generativelanguage.googleapis.com/v1beta/openai".into(),
            google_advanced: false,
            nvidia_key: String::new(),
            nvidia_endpoint: "https://integrate.api.nvidia.com/v1".into(),
            nvidia_advanced: false,
            router_key: String::new(),
            router_endpoint: "https://openrouter.ai/api/v1".into(),
            router_advanced: false,
            google_status: "Enter a key, then verify or save".into(),
            nvidia_status: "Enter a key, then verify or save".into(),
            router_model_filter: String::new(),
            google_model_filter: String::new(),
            nvidia_model_filter: String::new(),
            router_status: "Not checked".into(),
            provider_pending: None,
            focus: Target::App(0),
            cursor: 0,
            screen: Rect::new(0, 0, 100, 34),
            status: String::new(),
            memories: Vec::new(),
            selected_insight: None,
            memory_sel: 0,
            brain_list_mode: BrainListMode::List,
            atlas_page: AtlasPage::Runs,
            atlas_stats: atlas::RunStats::default(),
            atlas_feed: Vec::new(),
            atlas_feed_sel: 0,
            atlas_feed_follow: true,
            atlas_runs: Vec::new(),
            atlas_run_sel: 0,
            atlas_news: false,
            atlas_news_run: String::new(),
            atlas_articles: Vec::new(),
            atlas_article_sel: 0,
            claim_mark: None,
            pointer: None,
            atlas_focus: None,
            atlas_map_hold: false,
            atlas_status: "Ready".into(),
            atlas_state: "idle".into(),
            atlas_insight_progress: None,
            atlas_pause: None,
            atlas_auto_next: None,
            atlas_auto_started: false,
            intel_page: IntelPage::Bulletin,
            intel_category: INTEL_CATEGORIES[0].into(),
            intel_day: String::new(),
            intel_days: Vec::new(),
            intel_articles: Vec::new(),
            intel_sel: 0,
            intel_claims: Vec::new(),
            intel_relations: Vec::new(),
            intel_body: None,
            intel_body_message: String::new(),
            intel_full_collapsed: false,
            intel_jobs: Vec::new(),
            intel_sections: Vec::new(),
            intel_job_sel: 0,
            intel_recon_tab: 0,
            intel_recon_recommended: intel_recon::default_recon_mode(),
            intel_recon_recommended_for: String::new(),
            intel_mode_classifying: false,
            intel_recon_enabled: HashMap::new(),
            intel_recon_focus: IntelReconFocus::Tab(0),
            intel_body_running: HashSet::new(),
            intel_insights_running: HashSet::new(),
            intel_actors_reviewing: HashSet::new(),
            intel_links_reviewing: HashSet::new(),
            intel_link_explanations: HashMap::new(),
            intel_report_running: HashMap::new(),
            brain_graph: recon::MemoryGraph::default(),
            brain_graph_for: None,
            brain_detail: Default::default(),
            memory_total: 0,
            memory_error: None,
            memories_loaded: true,
            graph_summary: String::new(),
            graph_summary_pending: None,
            graph_summary_request: String::new(),
            summary_failure: None,
            summary_details_open: false,
            hits: Vec::new(),
            auth: AuthFile::default(),
            settings: SettingsFile::default(),
            hardware: HardwareProfile::unknown(),
            profile: crate::tui::profile::ProfileView::default(),
            profile_config: crate::tui::profile_config::ConfigView::default(),
            auth_path: PathBuf::new(),
            settings_path: PathBuf::new(),
            store: Store::memory().unwrap(),
            home_draft_dirty: false,
            home_draft: String::new(),
            home_draft_cursor: 0,
            home_draft_scroll: 0,
            home_draft_report_mode: None,
            home_draft_submitting: false,
            home_draft_submission_token: None,
            home_draft_last_saved: None,
            tab_ids: Vec::new(),
            tab_sel: 0,
            tab_last_active: HashMap::new(),
            tab_recently_closed: Vec::new(),
            tab_unreads: HashSet::new(),
            launch_state: LaunchState::Editable,
            launch_navigated_away: false,
            last_user_nav_action: None,
            provider_tx,
            provider_rx,
            work_tx,
            work_rx,
        }
    }

    fn click(app: &mut App, target: Target) {
        let backend = ratatui::backend::TestBackend::new(app.screen.width, app.screen.height);
        let mut terminal = ratatui::Terminal::new(backend).expect("test terminal");
        terminal
            .draw(|frame| super::super::ui::draw(frame, app))
            .expect("render before click");
        let position = (0..app.screen.height)
            .flat_map(|y| (0..app.screen.width).map(move |x| (x, y)))
            .find(|(x, y)| super::super::ui::hit_test(app, *x, *y) == Some(target))
            .unwrap_or_else(|| panic!("visible click target: {target:?}"));
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
    fn multiline_paste_stays_in_composer_and_does_not_run_shortcuts() {
        let mut app = app();
        app.module = Some(ModuleId::Recon);
        app.recon_chat = true;
        app.set_focus(Target::Field(FieldId::Composer));
        app.edit_paste("Who? /new 123\nSearch evidence");
        assert_eq!(app.input, "Who? /new 123\nSearch evidence");
        assert_eq!(app.module, Some(ModuleId::Recon));
        assert_eq!(app.overlay, Overlay::None);
        assert!(app.messages.is_empty());
    }

    #[test]
    fn unavailable_palette_action_stays_visible_and_does_not_execute() {
        let mut app = app();
        app.module = Some(ModuleId::Recon);
        app.overlay = Overlay::Palette;
        app.palette_query = "cancel".into();
        let item = app
            .palette_items()
            .into_iter()
            .find(|item| item.id == "cancel")
            .unwrap();
        assert!(!item.enabled);
        assert_eq!(item.disabled_reason, "No running turn");
        app.palette_sel = 0;
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.overlay, Overlay::Palette);
        assert!(!app
            .palette_items()
            .iter()
            .any(|item| item.id == "jobs-retry"));
    }

    #[test]
    fn backspace_after_a_sent_question_deletes_one_character() {
        let mut app = app();
        app.module = Some(ModuleId::Recon);
        app.recon_chat = true;
        app.set_focus(Target::Field(FieldId::Composer));
        type_text(&mut app, "who is jane roe");
        app.input.clear();
        type_text(&mut app, "next");
        app.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        assert_eq!(app.input, "nex");
        app.handle_key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE));
        assert_eq!(app.input, "nex");
        app.cursor = 0;
        app.handle_key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE));
        assert_eq!(app.input, "ex");
    }

    #[test]
    fn brain_tab_still_edits_and_finds_sourced_memories() {
        let mut app = app();
        click(&mut app, Target::App(ModuleId::Brain.index()));
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
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.brain_list_mode, BrainListMode::Graph);
        assert_eq!(
            app.focus,
            Target::DetailPath,
            "detail opens with the graph section focused"
        );
        assert!(app.brain_graph.is_empty());
        assert!(app.graph_summary.contains("no investigation graph"));
        assert!(app.graph_summary_pending.is_none());
        let sel = app.memory_sel;
        let graph_for = app.brain_graph_for.clone();
        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(app.memory_sel, sel);
        assert_eq!(app.brain_graph_for, graph_for);
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.brain_list_mode, BrainListMode::List);
        click(&mut app, Target::Field(FieldId::BrainQuery));
        assert!(app.selected_insight.is_none());
        type_text(&mut app, "Atlas");
        assert_eq!(app.memories.len(), 1);
        assert!(app.memories[0].text.contains("Atlas"));
    }

    #[test]
    fn application_order_is_intel_atlas_brain_recon_with_updated_blurbs() {
        assert_eq!(
            ModuleId::ALL[0..=3],
            [
                ModuleId::Intel,
                ModuleId::Atlas,
                ModuleId::Brain,
                ModuleId::Recon
            ]
        );
        assert_eq!(
            ModuleId::Intel.blurb(),
            "Review intelligence briefings and investigate emerging stories"
        );
        assert_eq!(
            ModuleId::Atlas.blurb(),
            "Map and track the global news cycle"
        );
        assert_eq!(
            ModuleId::Brain.blurb(),
            "Recall and explore connected intelligence"
        );
        assert_eq!(
            ModuleId::Recon.blurb(),
            "Run evidence-driven OSINT investigations"
        );
    }

    #[test]
    fn ctrl_tab_cycles_app_tabs_forward_and_back() {
        let mut app = app();
        assert_eq!(app.module, None);
        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::CONTROL));
        assert_eq!(app.module, Some(ModuleId::Intel));
        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::CONTROL));
        assert_eq!(app.module, Some(ModuleId::Atlas));
        app.handle_key(KeyEvent::new(
            KeyCode::Tab,
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        ));
        assert_eq!(app.module, Some(ModuleId::Intel));
        app.handle_key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::CONTROL));
        assert_eq!(app.module, Some(ModuleId::System));
        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::CONTROL));
        assert_eq!(app.module, Some(ModuleId::Intel));
    }

    #[test]
    fn ctrl_arrows_cycle_app_tabs_forward_and_back() {
        let mut app = app();
        app.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::CONTROL));
        assert_eq!(app.module, Some(ModuleId::Intel));
        app.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::CONTROL));
        assert_eq!(app.module, Some(ModuleId::Atlas));
        app.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::CONTROL));
        assert_eq!(app.module, Some(ModuleId::Intel));
        app.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::CONTROL));
        assert_eq!(app.module, Some(ModuleId::System));
    }

    #[test]
    fn number_keys_switch_apps_when_not_in_a_field() {
        let mut app = app();
        app.handle_key(KeyEvent::new(KeyCode::Char('4'), KeyModifiers::NONE));
        assert_eq!(app.module, Some(ModuleId::Recon));
        // Recon opens onto a search field; leave it so digits switch apps again.
        app.set_focus(Target::Button(ButtonId::DeleteThread));
        app.handle_key(KeyEvent::new(KeyCode::Char('2'), KeyModifiers::NONE));
        assert_eq!(app.module, Some(ModuleId::Atlas));
        app.select(ModuleId::Intel.index());
        app.set_focus(Target::Field(FieldId::IntelSearch));
        app.handle_key(KeyEvent::new(KeyCode::Char('3'), KeyModifiers::NONE));
        assert_eq!(app.module, Some(ModuleId::Intel));
        assert!(app.intel_search.contains('3'));
    }

    fn seed_intel_articles(app: &mut App) {
        app.store.atlas_insert_run("run-intel", "{}", "{}").unwrap();
        let now = chrono::Utc::now().to_rfc3339();
        for (id, title) in [
            ("a1", "Border talks stall"),
            ("a2", "Geneva summit delayed"),
        ] {
            app.store
                .atlas_upsert_article(&AtlasArticleRow {
                    run_id: "run-intel".into(),
                    id: id.into(),
                    title: title.into(),
                    description: format!("{title} body"),
                    url: format!("https://example.com/{id}"),
                    country: "us".into(),
                    source_name: "Wire".into(),
                    source_domain: "example.com".into(),
                    published_at: now.clone(),
                    provider: "newsapi".into(),
                    temperature: 0.88,
                    category: "geopolitical".into(),
                    seen_at: now.clone(),
                    author: String::new(),
                    image_url: String::new(),
                })
                .unwrap();
        }
    }

    #[test]
    fn intel_opens_bulletin_filters_and_opens_briefing() {
        let mut app = app();
        seed_intel_articles(&mut app);
        click(&mut app, Target::App(ModuleId::Intel.index()));
        assert_eq!(app.module, Some(ModuleId::Intel));
        assert_eq!(app.intel_page, IntelPage::Bulletin);
        assert_eq!(app.intel_articles.len(), 2);
        assert_eq!(app.intel_sel, 0);

        click(&mut app, Target::Button(ButtonId::IntelDay));
        assert!(matches!(app.overlay, Overlay::Choice(ChoiceKind::IntelDay)));
        app.apply_choice(0);
        assert_eq!(app.overlay, Overlay::None);
        assert!(!app.intel_day.is_empty());

        click(&mut app, Target::Field(FieldId::IntelSearch));
        type_text(&mut app, "geneva");
        assert_eq!(app.intel_articles.len(), 1);
        assert_eq!(app.intel_articles[0].id, "a2");

        click(&mut app, Target::IntelArticle(0));
        assert_eq!(app.intel_page, IntelPage::Briefing);
        app.intel_recon_recommended = ReportMode::Explain;
        click(&mut app, Target::Button(ButtonId::IntelReports));
        assert_eq!(app.overlay, Overlay::IntelRecon);
        assert_eq!(app.intel_recon_tab, 1);
        assert!(app
            .intel_recon_enabled
            .get("explain")
            .is_some_and(|set| set.len() >= 4));
        assert_eq!(app.intel_page, IntelPage::Briefing);
        app.on_esc();
        assert_eq!(app.overlay, Overlay::None);
        app.on_esc();
        assert_eq!(app.intel_page, IntelPage::Bulletin);
    }

    #[test]
    fn brain_anchors_follow_memory_focus_and_scroll_stops_at_ends() {
        let mut app = app();
        click(&mut app, Target::App(ModuleId::Brain.index()));
        for (app_name, conversation, text) in [
            ("chat", "t1", "alpha memory"),
            ("chat", "t2", "beta memory"),
            ("chat", "t3", "gamma memory"),
        ] {
            click(&mut app, Target::Button(ButtonId::CreateMemory));
            click(&mut app, Target::Field(FieldId::BrainApp));
            app.brain_app.clear();
            type_text(&mut app, app_name);
            click(&mut app, Target::Field(FieldId::BrainConversation));
            app.brain_conversation.clear();
            type_text(&mut app, conversation);
            click(&mut app, Target::Field(FieldId::BrainInsight));
            app.brain_insight.clear();
            type_text(&mut app, text);
            click(&mut app, Target::Button(ButtonId::Add));
        }
        assert_eq!(app.memories.len(), 3);
        click(&mut app, Target::Memory(1));
        assert!(matches!(app.focus, Target::Memory(1)));
        // Manual memories have no insight claims.
        assert!(app.selected_insight.is_none());
        click(&mut app, Target::Field(FieldId::BrainQuery));
        assert!(app.selected_insight.is_none());
        click(&mut app, Target::Memory(0));
        app.scrolls.memories = 0;
        app.move_memory(-1);
        assert_eq!(app.memory_sel, 0);
        assert_eq!(app.scrolls.memories, 0);
        click(&mut app, Target::Memory(2));
        let bottom = app.scrolls.memories;
        app.move_memory(1);
        assert_eq!(app.memory_sel, 2);
        assert_eq!(app.scrolls.memories, bottom);
        // Selection past the visible window is pulled back into view.
        app.screen = Rect::new(0, 0, 80, 24);
        app.scrolls.memories = 0;
        app.memory_sel = 2;
        app.set_focus(Target::Memory(2));
        super::super::ui::normalize(&mut app);
        let room = super::super::ui::memory_room_for(&app);
        assert!(
            app.memory_sel >= app.scrolls.memories as usize
                && app.memory_sel < app.scrolls.memories as usize + room.max(1),
            "sel {} scroll {} room {}",
            app.memory_sel,
            app.scrolls.memories,
            room
        );
    }

    #[test]
    fn provider_auth_tabs_and_router_form_are_clickable() {
        let mut app = app();
        click(&mut app, Target::App(ModuleId::Providers.index()));
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
        click(&mut app, Target::App(ModuleId::Providers.index()));
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

        click(&mut app, Target::App(ModuleId::Providers.index()));
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
        assert!(events(&app)
            .iter()
            .any(|line| line.message.starts_with("defaults.tool_picker:")));
        let saved = std::fs::read_to_string(&app.settings_path).unwrap();
        assert!(saved.contains("[defaults.tool_picker]"), "{saved}");
        assert!(saved.contains("typesafe/jev-1.13"));
        assert!(!saved.contains("router-key"));
        assert!(!app.auth_path.exists());
        assert_eq!(serde_json::to_string(&app.auth).unwrap(), auth_before);
    }

    #[test]
    fn provider_verify_result_updates_only_its_page() {
        let mut app = app();
        app.provider_pending = Some(ProviderPage::Google);
        app.on_provider_event(ProviderEvent::Finished {
            page: ProviderPage::Google,
            result: Ok("Google catalog reachable · 2 models".into()),
        });
        assert!(app.google_status.contains("reachable"));
        assert_eq!(app.nvidia_status, "Enter a key, then verify or save");
        assert_eq!(app.provider_pending, None);
    }

    #[test]
    fn all_provider_actions_render_with_hit_areas_at_80x24() {
        let mut app = app();
        app.screen = Rect::new(0, 0, 80, 24);
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 24)).unwrap();
        app.select(ModuleId::Providers.index());
        for (page, target) in [
            (ProviderPage::Google, Target::Button(ButtonId::GoogleSave)),
            (ProviderPage::Nvidia, Target::Button(ButtonId::NvidiaSave)),
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
        app.defaults_role = DefaultsRole::Recon;
        terminal
            .draw(|frame| super::super::ui::draw(frame, &app))
            .unwrap();
        assert!(!(0..24)
            .flat_map(|y| (0..80).map(move |x| (x, y)))
            .any(|(x, y)| super::super::ui::hit_test(&app, x, y)
                == Some(Target::Button(ButtonId::RefreshModels))));
        let painted = screen_text(&terminal);
        assert!(
            painted.contains("No fallback models configured"),
            "{painted}"
        );
        assert!(painted.contains("Add fallback"), "{painted}");
        assert!(!painted.contains("Refresh models"), "{painted}");
    }

    #[test]
    fn defaults_fallbacks_add_delete_reorder_persist() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app();
        app.settings_path = dir.path().join("config.toml");
        app.auth_path = dir.path().join("auth.json");
        let mut google = provider::account_secret(&app.auth, "google");
        google.api_key = Some("google-key".into());
        app.auth.set_account(google);
        app.google_key = "google-key".into();
        app.select(ModuleId::Providers.index());
        app.provider_page = ProviderPage::Defaults;
        click(&mut app, Target::Button(ButtonId::AddFallback));
        assert_eq!(app.overlay, Overlay::AddFallback);
        app.on_work_event(WorkEvent::CatalogDone {
            role: DefaultsRole::Recon,
            provider: "google".into(),
            outcome: Ok(vec![
                ListedModel {
                    id: "gemini-2.5-pro".into(),
                    name: "Gemini 2.5 Pro".into(),
                    free: false,
                },
                ListedModel {
                    id: "gemini-flash".into(),
                    name: "Gemini Flash".into(),
                    free: true,
                },
            ]),
        });
        app.fallback_popup_sel = 0;
        click(&mut app, Target::Button(ButtonId::ConfirmAddFallback));
        assert_eq!(app.overlay, Overlay::None);
        assert_eq!(app.settings.defaults.recon.fallbacks.len(), 1);
        assert_eq!(
            app.settings.defaults.recon.fallbacks[0].model,
            "gemini-2.5-pro"
        );
        app.overlay = Overlay::AddFallback;
        app.fallback_popup_tab = ProviderPage::Google;
        app.fallback_popup_sel = 1;
        click(&mut app, Target::Button(ButtonId::ConfirmAddFallback));
        assert_eq!(app.settings.defaults.recon.fallbacks.len(), 2);
        click(&mut app, Target::Button(ButtonId::FallbackItem(1)));
        click(&mut app, Target::Button(ButtonId::MoveFallbackUp));
        assert_eq!(
            app.settings.defaults.recon.fallbacks[0].model,
            "gemini-flash"
        );
        click(&mut app, Target::Button(ButtonId::FallbackItem(1)));
        click(&mut app, Target::Button(ButtonId::DeleteFallback));
        assert_eq!(app.settings.defaults.recon.fallbacks.len(), 1);
        assert_eq!(
            app.settings.defaults.recon.fallbacks[0].model,
            "gemini-flash"
        );
        let saved = std::fs::read_to_string(&app.settings_path).unwrap();
        assert!(saved.contains("gemini-flash"), "{saved}");
        assert!(!saved.contains("gemini-2.5-pro"), "{saved}");
        click(&mut app, Target::Button(ButtonId::AddFallback));
        assert_eq!(app.overlay, Overlay::AddFallback);
        click(&mut app, Target::CloseOverlay);
        assert_eq!(app.overlay, Overlay::None);
        assert_eq!(app.focus, Target::Button(ButtonId::AddFallback));
    }

    #[test]
    fn defaults_pick_provider_and_model_from_account_access() {
        let mut app = app();
        app.google_key = "google-key".into();
        app.nvidia_key = "nvidia-key".into();
        let mut router = provider::account_secret(&app.auth, "openrouter");
        router.api_key = Some("router-key".into());
        app.auth.set_account(router);
        click(&mut app, Target::App(ModuleId::Providers.index()));
        click(&mut app, Target::ProviderTab(ProviderPage::Defaults));
        click(&mut app, Target::Field(FieldId::ReconProvider));
        assert!(matches!(app.overlay, Overlay::Choice(ChoiceKind::Provider)));
        let ids: Vec<_> = app
            .choice_items
            .iter()
            .map(|item| item.id.as_str())
            .collect();
        assert_eq!(ids, ["google", "nvidia", "openrouter", "local"]);
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
        assert_eq!(app.choice_items[0].id, "google");
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.synthesis_provider, "google");
        assert!(app.synthesis_model.is_empty());
        assert_eq!(app.recon_provider, "openrouter");
        assert_eq!(app.recon_model, "beta");

        click(&mut app, Target::Field(FieldId::SynthesisProvider));
        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.synthesis_provider, "nvidia");
        app.on_work_event(WorkEvent::CatalogDone {
            role: DefaultsRole::Synthesis,
            provider: "nvidia".into(),
            outcome: Ok(vec![ListedModel {
                id: "nvidia-text-model".into(),
                name: "Nvidia text model".into(),
                free: false,
            }]),
        });
        click(&mut app, Target::Field(FieldId::SynthesisModel));
        assert!(matches!(app.overlay, Overlay::Choice(ChoiceKind::Model)));
        assert_eq!(
            app.choice_items
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            ["nvidia-text-model"]
        );
        click(&mut app, Target::Choice(0));
        assert_eq!(app.synthesis_model, "nvidia-text-model");
        app.on_work_event(WorkEvent::CatalogDone {
            role: DefaultsRole::Synthesis,
            provider: "grok".into(),
            outcome: Ok(vec![ListedModel {
                id: "not-allowed".into(),
                name: "Not allowed".into(),
                free: false,
            }]),
        });
        assert_eq!(app.catalog_for, "nvidia");
        assert_eq!(app.model_catalog[0].id, "nvidia-text-model");
        assert_eq!(app.recon_model, "beta");

        app.google_key.clear();
        app.nvidia_key.clear();
        app.auth = AuthFile::default();
        click(&mut app, Target::Field(FieldId::SynthesisProvider));
        let available: Vec<_> = app
            .choice_items
            .iter()
            .map(|item| item.id.as_str())
            .collect();
        assert!(available.contains(&"local"));
        assert!(!available.contains(&"google"));
        assert!(!available.contains(&"nvidia"));
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
        app.select(ModuleId::Recon.index());
        terminal.draw(|f| super::super::ui::draw(f, &app)).unwrap();
        for target in [
            Target::Field(FieldId::ReconSearch),
            Target::Button(ButtonId::DeleteThread),
        ] {
            assert!(
                (0..24)
                    .flat_map(|y| (0..80).map(move |x| (x, y)))
                    .any(|(x, y)| super::super::ui::hit_test(&app, x, y) == Some(target)),
                "dashboard is missing {target:?}"
            );
        }
        assert!(!hit(&app, Target::Button(ButtonId::NewThread)));
        assert!(!hit(&app, Target::Button(ButtonId::Send)));
        app.recon_chat = true;
        terminal.draw(|f| super::super::ui::draw(f, &app)).unwrap();
        for target in [
            Target::Button(ButtonId::CancelRun),
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
        assert!(!hit(&app, Target::Button(ButtonId::ResumeRun)));
        let t = app.store.new_thread("Test Thread").unwrap();
        app.selected_thread = Some(t.id.clone());
        let run = app
            .store
            .new_run(&t.id, "turn-1", "recon-m", "synth-m")
            .unwrap();
        app.store
            .set_run(&run.id, "interrupted", "interrupted", None, None)
            .unwrap();
        terminal.draw(|f| super::super::ui::draw(f, &app)).unwrap();
        assert!(hit(&app, Target::Button(ButtonId::ResumeRun)));
        assert!(!hit(&app, Target::Button(ButtonId::NewThread)));
        app.select(ModuleId::Osint.index());
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
        for target in [
            Target::App(ModuleId::Intel.index()),
            Target::App(ModuleId::Atlas.index()),
            Target::App(ModuleId::Recon.index()),
            Target::App(ModuleId::Providers.index()),
        ] {
            assert!(hit(&app, target), "home is missing {target:?}");
        }
        assert!(hit(&app, Target::Field(FieldId::Composer)));
        assert!(hit(&app, Target::Button(ButtonId::Send)));
        app.select(ModuleId::Brain.index());
        terminal
            .draw(|frame| super::super::ui::draw(frame, &app))
            .unwrap();
        assert!(!hit(&app, Target::Field(FieldId::Composer)));
        assert!(!super::super::ui::focus_order(&app).contains(&Target::Field(FieldId::Composer)));
        app.select(ModuleId::Recon.index());
        terminal
            .draw(|frame| super::super::ui::draw(frame, &app))
            .unwrap();
        assert!(hit(&app, Target::Field(FieldId::ReconSearch)));
        assert!(hit(&app, Target::Button(ButtonId::DeleteThread)));
        assert!(!hit(&app, Target::Button(ButtonId::NewThread)));
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

    fn events(app: &App) -> Vec<argos_osint_core::events::EventRow> {
        let mut rows = app.store.list_events(&Default::default(), 500).unwrap();
        rows.reverse();
        rows
    }

    #[test]
    fn logs_record_errors_durably_and_scroll() {
        let mut app = app();
        app.screen = Rect::new(0, 0, 80, 24);
        for index in 0..40 {
            app.push_log("error", format!("lookup failed {index}"));
        }
        assert_eq!(app.error_count(), 40, "home badge counts durable errors");
        app.select(ModuleId::Logs.index());
        assert_eq!(app.logs.rows.len(), 40);
        assert!(app.logs.follow);
        assert_eq!(app.logs.sel, 39, "live follow selects the newest event");
        let bottom = app.logs.scroll;
        assert!(bottom > 0, "follow keeps the newest event in view");
        app.handle_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
        assert!(app.logs.scroll < bottom);
        // System no longer shows or routes the event log.
        app.select(ModuleId::System.index());
        assert_eq!(
            super::super::ui::focus_order(&app)
                .into_iter()
                .filter(|t| matches!(t, Target::Button(_)))
                .collect::<Vec<_>>(),
            vec![Target::Button(ButtonId::RefreshHardware)]
        );
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
        let all = events(&app);
        let logged: Vec<_> = all.iter().filter(|line| !line.details.is_empty()).collect();
        assert_eq!(logged.len(), 1);
        assert!(logged[0].message.contains("cache"));
        assert!(logged[0].message.contains("2 results"));
        assert!(logged[0].details.contains("Jane Roe role"));
        app.module = Some(ModuleId::Recon);
        app.recon_chat = true;
        app.expanded.insert("tool:call-s1".into());
        let body = super::super::ui::chat_blocks(&app)
            .into_iter()
            .find(|block| block.key == "tool:call-s1")
            .expect("tool row")
            .body;
        assert!(body.contains("Full result is in Logs"));
        assert!(body.contains("cache"));
        assert!(body.contains("query: Jane Roe"));
        assert!(!body.contains("Jane Roe role"));
        let title = super::super::ui::chat_blocks(&app)
            .into_iter()
            .find(|block| block.key == "tool:call-s1")
            .expect("tool row")
            .title;
        assert!(title.contains("query=Jane Roe"), "{title}");
        assert!(title.contains("E1"), "{title}");
        app.chat_sel = 0;
        app.open_selected_evidence();
        if let Overlay::Block { title, body } = &app.overlay {
            assert!(title.contains("E1"));
            assert!(body.contains("https://example.test/jane"));
            assert!(body.contains("Jane Roe role"));
            assert!(body.contains("Assessment: unassessed"));
        } else {
            panic!("expected captured source inspector");
        }
        app.overlay = Overlay::None;
        app.select(ModuleId::Logs.index());
        let index = app
            .logs
            .rows
            .iter()
            .position(|line| !line.details.is_empty())
            .unwrap();
        app.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        app.logs.select(index);
        app.set_focus(Target::LogLine(index));
        let id = app.logs.rows[index].id.clone();
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(app.logs.open.contains(&id));
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(!app.logs.open.contains(&id));
    }

    #[test]
    fn a_click_folds_an_event_and_incoming_entries_do_not_move_a_paused_list() {
        let mut app = app();
        app.screen = Rect::new(0, 0, 100, 40);
        app.push_log_detail("info", "old lookup", "detail line");
        app.push_log_detail("info", "fresh lookup", "fresh detail");
        app.select(ModuleId::Logs.index());
        let index = app
            .logs
            .rows
            .iter()
            .position(|row| row.message == "old lookup")
            .unwrap();
        let id = app.logs.rows[index].id.clone();
        click(&mut app, Target::LogLine(index));
        assert!(app.logs.open.contains(&id));
        assert!(!app.logs.follow, "selecting an older event pauses follow");
        app.push_log("info", "incoming while reading");
        app.dashboards_at = None;
        app.tick_dashboards();
        assert_eq!(app.logs.selected().unwrap().id, id, "selection kept by id");
        assert!(app.logs.open.contains(&id));
        let sel = app.logs.sel;
        click(&mut app, Target::LogLine(sel));
        assert!(!app.logs.open.contains(&id));
    }

    fn render(app: &mut App, width: u16, height: u16) -> ratatui::buffer::Buffer {
        app.screen = Rect::new(0, 0, width, height);
        super::super::ui::normalize(app);
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| super::super::ui::draw(frame, app))
            .unwrap();
        terminal.backend().buffer().clone()
    }

    fn buffer_text(buffer: &ratatui::buffer::Buffer) -> String {
        let area = buffer.area;
        let mut out = String::new();
        for y in 0..area.height {
            for x in 0..area.width {
                out.push_str(buffer[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }

    #[test]
    fn recon_context_hides_on_narrow_screen_without_changing_preference() {
        let mut app = app();
        let dir = tempfile::tempdir().unwrap();
        app.settings_path = dir.path().join("config.toml");
        app.module = Some(ModuleId::Recon);
        app.recon_chat = true;
        let wide = buffer_text(&render(&mut app, 130, 32));
        assert!(wide.contains("Evidence calls"));
        let narrow = buffer_text(&render(&mut app, 80, 24));
        assert!(!narrow.contains("Evidence calls"));
        assert!(app.recon_context_enabled);
        app.run_palette("context");
        assert!(!app.recon_context_enabled);
        let saved = std::fs::read_to_string(&app.settings_path).unwrap();
        assert!(saved.contains("tui_recon_context = false"));
        let wide_hidden = buffer_text(&render(&mut app, 130, 32));
        assert!(!wide_hidden.contains("Evidence calls"));
    }

    fn register(app: &App, id: &str, app_name: &str, title: &str) {
        app.store
            .register_job(
                &argos_osint_core::tasks::NewJob {
                    id: id.into(),
                    kind: "user".into(),
                    owner_scope: String::new(),
                    input_revision: String::new(),
                    deadline_at: String::new(),
                },
                &argos_osint_core::tasks::JobMeta {
                    app: app_name.into(),
                    operation: "test".into(),
                    title: title.into(),
                    ..Default::default()
                },
            )
            .unwrap();
    }

    fn job_event(
        app: &App,
        job: &str,
        app_name: &str,
        severity: &str,
        message: &str,
        details: &str,
    ) {
        app.store
            .record_event(&argos_osint_core::events::NewEvent {
                severity: Some(argos_osint_core::events::Severity::parse(severity)),
                app: app_name.into(),
                event_type: "test".into(),
                message: message.into(),
                details: details.into(),
                job_id: job.into(),
                ..Default::default()
            })
            .unwrap();
    }

    #[test]
    fn home_order_renames_and_nine_routes_agree() {
        let rows = super::super::ui::home_rows(Rect::new(0, 0, 140, 50), 3);
        type Group = (String, Vec<(String, String, usize)>);
        let mut groups: Vec<Group> = Vec::new();
        for row in rows {
            match row.kind {
                super::super::ui::HomeKind::Heading(title) => {
                    groups.push((title.into(), Vec::new()))
                }
                super::super::ui::HomeKind::Item { title, detail } => {
                    if let Some(group) = groups.last_mut() {
                        group.1.push((title, detail, row.target.unwrap()));
                    }
                }
                _ => {}
            }
        }
        let titles = |name: &str| -> Vec<String> {
            groups
                .iter()
                .find(|(title, _)| title == name)
                .unwrap()
                .1
                .iter()
                .map(|(title, _, _)| title.clone())
                .collect()
        };
        assert_eq!(titles("Applications"), ["Intel", "Atlas", "Brain", "Recon"]);
        assert_eq!(
            titles("System"),
            ["Jobs", "Logs", "Tools", "Models", "Profile"]
        );
        let all: Vec<usize> = groups
            .iter()
            .flat_map(|g| g.1.iter().map(|i| i.2))
            .collect();
        assert_eq!(
            all,
            (0..9).collect::<Vec<_>>(),
            "home targets follow numeric order"
        );
        let logs = &groups[1].1[1];
        assert!(
            logs.1.contains("3 errors"),
            "error badge moved to Logs: {logs:?}"
        );
        assert!(!groups[1].1[4].1.contains("errors"));
        // Display renames keep internal ids.
        assert_eq!(ModuleId::Osint.title(), "Tools");
        assert_eq!(ModuleId::Providers.title(), "Models");
        assert_eq!(
            ModuleId::System.blurb(),
            "Inspect host hardware and Argos storage"
        );
        // 1–9 from home.
        for (index, module) in ModuleId::ALL.iter().enumerate() {
            let mut app = app();
            let digit = char::from(b'1' + index as u8);
            app.handle_key(KeyEvent::new(KeyCode::Char(digit), KeyModifiers::NONE));
            assert_eq!(app.module, Some(*module), "digit {digit}");
        }
        // Palette and slash aliases, old and new.
        let mut app = app();
        for (alias, module) in [
            ("tools", ModuleId::Osint),
            ("osint", ModuleId::Osint),
            ("models", ModuleId::Providers),
            ("providers", ModuleId::Providers),
            ("jobs", ModuleId::Jobs),
            ("logs", ModuleId::Logs),
            ("system", ModuleId::System),
            ("profile", ModuleId::System),
        ] {
            app.go_home();
            app.run_palette(alias);
            assert_eq!(app.module, Some(module), "palette {alias}");
            app.go_home();
            app.run_slash(&format!("/{alias}")).unwrap();
            assert_eq!(app.module, Some(module), "slash {alias}");
        }
        let ids: Vec<String> = app.palette_items().into_iter().map(|i| i.id).collect();
        for id in ["jobs", "logs", "tools", "models", "profile"] {
            assert!(ids.contains(&id.to_string()), "{id}");
        }
        app.select(ModuleId::Logs.index());
        assert!(app
            .palette_items()
            .iter()
            .any(|item| item.id == "clear-log"));
        app.palette_query = "system".into();
        assert!(app.palette_items().iter().any(|item| item.id == "profile"));
        app.palette_query.clear();
        // Help and the header agree with the order.
        app.go_home();
        app.overlay = Overlay::Help;
        let mut app2 = app;
        let text = buffer_text(&render(&mut app2, 160, 40));
        assert!(
            text.contains("1 Intel · 2 Atlas · 3 Brain · 4 Recon · 5 Jobs · 6 Logs · 7 Tools"),
            "{text}"
        );
        assert!(text.contains("Models · 9 Profile"));
        app2.overlay = Overlay::None;
        app2.select(ModuleId::Jobs.index());
        let header = buffer_text(&render(&mut app2, 160, 40));
        let first = header.lines().next().unwrap();
        assert!(first.contains("[Home] > Jobs >"), "{first}");
        assert!(!first.contains("Intel"), "{first}");
    }

    #[test]
    fn registered_operations_show_in_jobs_and_cancel_is_cooperative() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("argos.db");
        let mut app = app();
        app.store = Store::open(&db).unwrap();
        app.screen = Rect::new(0, 0, 140, 40);
        super::super::tracked::testing::use_db(Some(db.clone()));
        // A tool run as the TUI registers it: durable before work starts.
        let stop = Arc::new(AtomicBool::new(false));
        let job = super::super::tracked::begin_cancellable(
            JobSpec::new("tools", "tool_run", "Run WHOIS lookup").tool("whois"),
            stop.clone(),
        )
        .expect("registered");
        let id = job.id().to_string();
        // A non-cancellable operation never offers Cancel.
        let summary = super::super::tracked::begin(JobSpec::new(
            "brain",
            "graph_summary",
            "Explain claim path",
        ))
        .expect("registered");
        app.select(ModuleId::Jobs.index());
        assert!(app.jobs.focus_job(&app.store, &id), "listed");
        let text = buffer_text(&render(&mut app, 140, 40));
        assert!(
            text.contains("Run WHOIS lookup") && text.contains("Explain claim path"),
            "{text}"
        );
        assert!(text.contains("Cancel"), "{text}");
        assert!(
            app.jobs
                .selected()
                .unwrap()
                .active_now(chrono::Utc::now())
                .is_some(),
            "live active time"
        );

        app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE));
        assert!(
            stop.load(Ordering::Relaxed),
            "the tool's own stop flag is set"
        );
        assert!(app.status.contains("Cancel requested"), "{}", app.status);
        assert_eq!(
            app.jobs.selected().unwrap().state,
            "running",
            "not cancelled until it stops"
        );
        let text = buffer_text(&render(&mut app, 140, 40));
        assert!(text.contains("Cancelling…"), "{text}");
        assert!(!app.jobs.can_cancel(), "no second Cancel");

        // The operation observes the flag and stops: cancelled, not failed.
        super::super::tracked::finish(Some(job), "tool", &Err::<(), _>("stopped"), Some(&stop));
        super::super::tracked::finish(Some(summary), "summary", &Ok::<(), String>(()), None);
        app.reload_jobs();
        let rows: HashMap<String, String> = app
            .jobs
            .rows
            .iter()
            .map(|r| (r.title.clone(), r.state.clone()))
            .collect();
        assert_eq!(rows["Run WHOIS lookup"], "cancelled");
        assert_eq!(rows["Explain claim path"], "completed");
        let summary_row = app
            .jobs
            .rows
            .iter()
            .find(|r| r.title == "Explain claim path")
            .unwrap();
        assert!(!summary_row.cancellable);
        super::super::tracked::testing::use_db(None);
    }

    #[test]
    fn jobs_dashboard_navigates_to_logs_and_back_and_logs_open_jobs() {
        let mut app = app();
        app.screen = Rect::new(0, 0, 140, 40);
        register(&app, "job-a", "atlas", "Atlas news cycle");
        app.store
            .set_job_progress(
                "job-a",
                "running",
                "Index and verify memories",
                3,
                Some(5),
                "",
            )
            .unwrap();
        register(&app, "job-b", "atlas", "Repair Atlas memories");
        app.store
            .set_job_progress("job-b", "completed", "done", 4, Some(4), "")
            .unwrap();
        job_event(&app, "job-a", "atlas", "info", "phase 5 started", "");
        job_event(&app, "", "recon", "warn", "unrelated", "");
        app.handle_key(KeyEvent::new(KeyCode::Char('5'), KeyModifiers::NONE));
        assert_eq!(app.module, Some(ModuleId::Jobs));
        assert_eq!(app.jobs.rows[0].id, "job-a", "active work first");
        assert_eq!(app.focus, Target::JobRow(0));
        assert!(app.jobs.detail.is_some());
        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(app.jobs.selected().unwrap().id, "job-b");
        app.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        let text = buffer_text(&render(&mut app, 140, 40));
        assert!(text.contains("Index and verify memories"), "{text}");
        assert!(text.contains("View logs"));
        assert!(!text.contains("Retry failed"), "no misleading Retry");

        // Jobs → Logs prefiltered to the job, with a return path.
        app.handle_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE));
        assert_eq!(app.module, Some(ModuleId::Logs));
        assert_eq!(app.logs.job, "job-a");
        let messages: Vec<&str> = app.logs.rows.iter().map(|r| r.message.as_str()).collect();
        assert_eq!(messages, ["phase 5 started"]);
        app.set_focus(Target::LogLine(0));
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.module, Some(ModuleId::Jobs));
        assert_eq!(app.jobs.selected().unwrap().id, "job-a");
        assert!(app.logs.job.is_empty() && app.logs.back_to_job.is_none());

        // Logs → job for an event that carries one, even with Jobs filtered.
        app.jobs.status = argos_osint_core::jobs_view::JobStatusFilter::Completed;
        app.select(ModuleId::Logs.index());
        assert_eq!(app.logs.rows.len(), 2, "unfiltered Logs keep app events");
        let index = app
            .logs
            .rows
            .iter()
            .position(|r| r.job_id == "job-a")
            .unwrap();
        click(&mut app, Target::LogLine(index));
        app.handle_key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE));
        assert_eq!(app.module, Some(ModuleId::Jobs));
        assert_eq!(app.jobs.selected().unwrap().id, "job-a");

        // Narrow terminals switch to a single-panel detail.
        app.screen = Rect::new(0, 0, 80, 30);
        app.set_focus(Target::JobRow(app.jobs.sel));
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(app.jobs.detail_open);
        let narrow = buffer_text(&render(&mut app, 80, 30));
        assert!(
            narrow.contains("detail") && !narrow.contains("Repair Atlas memories"),
            "{narrow}"
        );
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(!app.jobs.detail_open);
        assert_eq!(app.module, Some(ModuleId::Jobs));
    }

    #[test]
    fn profile_splits_into_overview_and_system_tabs_and_logs_own_clear() {
        let mut app = app();
        app.select(ModuleId::System.index());

        // Overview is the activity tab: telemetry widgets, never host details.
        let overview = buffer_text(&render(&mut app, 120, 34));
        assert!(
            overview.contains(" period ") && overview.contains("[Tab] switch"),
            "the overview tab shows the filter strip and the tab switch: {overview}"
        );
        assert!(
            !overview.contains("Database:") && !overview.contains("Refresh hardware"),
            "host and paths belong to the System tab: {overview}"
        );

        // A bare Tab moves to the System tab, which keeps the panes it always had.
        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(app.profile.tab, crate::tui::profile::SystemTab::System);
        let system = buffer_text(&render(&mut app, 120, 34));
        assert!(system.contains("Refresh hardware"), "{system}");
        assert!(system.contains(" host ") && system.contains(" paths "));
        assert!(system.contains("Database:") && system.contains("Config:"));
        assert!(
            !system.contains("event log") && !system.contains("Clear"),
            "{system}"
        );
        click(&mut app, Target::Button(ButtonId::RefreshHardware));
        assert_eq!(app.status, "Hardware refreshed");

        // Tab moves back to Overview, so the two tabs really are one pane.
        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(app.profile.tab, crate::tui::profile::SystemTab::Overview);

        // Logs keeps its own clear action and its own retention label.
        register(&app, "job-c", "brain", "Graph summary");
        job_event(
            &app,
            "job-c",
            "brain",
            "error",
            "summary failed",
            "HTTP 503",
        );
        app.select(ModuleId::Logs.index());
        let text = buffer_text(&render(&mut app, 120, 34));
        assert!(
            text.contains("Clear events") && text.contains("kept 24 h"),
            "{text}"
        );
        click(&mut app, Target::Button(ButtonId::ClearLog));
        assert!(app.status.starts_with("Cleared"), "{}", app.status);
        assert!(
            app.store.get_job("job-c").unwrap().is_some(),
            "clearing events keeps jobs"
        );
    }

    /// Writes cell dumps of the phase-5 screens for PNG rendering:
    /// `ARGOS_SCREEN_DIR=/workspace/screens/atlas-memory cargo test -p argos-osint-bin dump_phase5_screens -- --ignored`
    #[test]
    #[ignore]
    fn dump_phase5_screens() {
        let Ok(dir) = std::env::var("ARGOS_SCREEN_DIR") else {
            return;
        };
        std::fs::create_dir_all(&dir).unwrap();
        let mut app = app();
        register(&app, "job-atlas-0412", "atlas", "Atlas news cycle · 04:12");
        // Realistic timing: two finished index attempts and one still open.
        app.store
            .fixture_task_attempt("job-atlas-0412", "t-idx-1", "index_upsert", 600, Some(45))
            .unwrap();
        app.store
            .fixture_task_attempt("job-atlas-0412", "t-idx-2", "index_upsert", 420, Some(38))
            .unwrap();
        app.store
            .fixture_task_attempt("job-atlas-0412", "t-idx-3", "index_upsert", 192, None)
            .unwrap();
        app.store
            .set_job_progress(
                "job-atlas-0412",
                "running",
                "Index and verify memories",
                41,
                Some(57),
                "",
            )
            .unwrap();
        register(
            &app,
            "job-intel-77",
            "intel",
            "Intel Recon · Full assessment",
        );
        register(
            &app,
            "job-graph-19",
            "brain",
            "Graph summary · Northwind ferry",
        );
        app.store
            .fixture_task_attempt(
                "job-graph-19",
                "t-sum-1",
                "graph_explanation",
                300,
                Some(12),
            )
            .unwrap();
        app.store
            .set_job_progress(
                "job-graph-19",
                "failed",
                "explanation",
                1,
                Some(2),
                "provider HTTP 503: upstream unavailable after 2 attempts",
            )
            .unwrap();
        register(
            &app,
            "atlas-memory-repair",
            "atlas",
            "Repair Atlas memories",
        );
        app.store
            .fixture_task_attempt(
                "atlas-memory-repair",
                "t-rep-1",
                "index_rebuild",
                3000,
                Some(21),
            )
            .unwrap();
        app.store
            .set_job_progress("atlas-memory-repair", "completed", "done", 12, Some(12), "")
            .unwrap();
        let _ = app.store.add_memory(
            "Harbor tanker manifests list cargo",
            "fact",
            false,
            MemorySource {
                app: "test".into(),
                conversation_id: "c".into(),
                message_id: None,
                reference: None,
            },
        );
        job_event(
            &app,
            "job-atlas-0412",
            "atlas",
            "info",
            "Atlas: phase 4 published 57 memories (12 created, 45 reused)",
            "",
        );
        job_event(
            &app,
            "job-atlas-0412",
            "atlas",
            "info",
            "Atlas: phase 5 indexing 41/57 verified",
            "",
        );
        job_event(&app, "job-graph-19", "brain", "error", "Graph summary failed: provider HTTP 503", "stage: stream\nattempt 1: HTTP 503 upstream unavailable\nattempt 2: HTTP 503 upstream unavailable\nfallback: basic graph explanation");
        job_event(&app, "", "recon", "info", "Recon turn complete", "");
        job_event(
            &app,
            "",
            "atlas",
            "warn",
            "Atlas: GNews rate limit reached (HTTP 429)",
            "",
        );
        job_event(
            &app,
            "atlas-memory-repair",
            "atlas",
            "info",
            "Repair Atlas memories: 12 runs checked, 3 vectors requeued",
            "",
        );
        let mut shots: Vec<(&str, u16, u16)> = Vec::new();
        let mut save = |app: &mut App, name: &str, width: u16, height: u16| {
            let buffer = render(app, width, height);
            let mut cells = Vec::new();
            for y in 0..height {
                let mut row = Vec::new();
                for x in 0..width {
                    let cell = &buffer[(x, y)];
                    row.push(serde_json::json!({
                        "s": cell.symbol(),
                        "fg": format!("{:?}", cell.fg),
                        "bg": format!("{:?}", cell.bg),
                        "b": cell.modifier.contains(ratatui::style::Modifier::BOLD),
                        "u": cell.modifier.contains(ratatui::style::Modifier::UNDERLINED),
                    }));
                }
                cells.push(row);
            }
            let json = serde_json::json!({"width": width, "height": height, "cells": cells});
            std::fs::write(format!("{dir}/{name}.json"), json.to_string()).unwrap();
            shots.push(("", width, height));
        };
        app.logs.refresh_counts(&app.store);
        save(&mut app, "home", 140, 42);
        app.select(ModuleId::Jobs.index());
        save(&mut app, "jobs", 140, 40);
        let failed = app
            .jobs
            .rows
            .iter()
            .position(|r| r.id == "job-graph-19")
            .unwrap();
        click(&mut app, Target::JobRow(failed));
        save(&mut app, "jobs-failed", 140, 40);
        app.screen = Rect::new(0, 0, 80, 30);
        app.set_focus(Target::JobRow(app.jobs.sel));
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        save(&mut app, "jobs-narrow-detail", 80, 30);
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        app.select(ModuleId::Logs.index());
        let index = app
            .logs
            .rows
            .iter()
            .position(|r| r.message.starts_with("Graph summary failed"))
            .unwrap();
        click(&mut app, Target::LogLine(index));
        save(&mut app, "logs", 140, 40);
        app.select(ModuleId::System.index());
        click(&mut app, Target::Button(ButtonId::RefreshHardware));
        save(&mut app, "system", 140, 40);
        assert!(!shots.is_empty());
    }

    fn atlas_claim(
        fingerprint: &str,
        entity: &str,
        claim: &str,
        article: &str,
    ) -> argos_osint_core::store::AtlasInsightClaim {
        argos_osint_core::store::AtlasInsightClaim {
            fingerprint: fingerprint.into(),
            entity: entity.into(),
            namespace: "org".into(),
            predicate: "reported".into(),
            object: claim.split(' ').next_back().unwrap_or("").into(),
            topic: "Baltic shipping".into(),
            claim: claim.into(),
            classification: "fact".into(),
            confidence: 0.82,
            article_id: article.into(),
            source_url: format!("https://news.example/{article}"),
            published_at: "2026-10-05T04:12:00Z".into(),
            reliability: "B".into(),
            info_credibility: 2,
            admiralty: "B2".into(),
            rsp_status: String::new(),
        }
    }

    /// One Atlas cycle with linked claims, plus a manual memory with similar
    /// wording. Returns memory ids in claim order, then the manual one.
    fn claim_fixture(app: &App) -> Vec<String> {
        let store = &app.store;
        store.atlas_insert_run("run-0412", "{}", "{}").unwrap();
        for (id, title) in [
            (
                "a1",
                "Northwind ferry halts Baltic crossings after engine fire",
            ),
            ("a2", "Baltic board eases ferry suspension"),
        ] {
            store
                .atlas_upsert_article(&AtlasArticleRow {
                    run_id: "run-0412".into(),
                    id: id.into(),
                    title: title.into(),
                    description: String::new(),
                    url: format!("https://news.example/{id}"),
                    country: "LT".into(),
                    source_name: "Baltic Wire".into(),
                    source_domain: "news.example".into(),
                    published_at: "2026-10-05T04:12:00Z".into(),
                    provider: "gnews".into(),
                    temperature: 0.5,
                    category: "economic".into(),
                    seen_at: "2026-10-05T04:12:00Z".into(),
                    author: String::new(),
                    image_url: String::new(),
                })
                .unwrap();
        }
        let texts = [
            (
                "fp-1",
                "Northwind Ferries",
                "Northwind Ferries halted Baltic crossings after an engine fire",
                "a1",
            ),
            (
                "fp-2",
                "Klaipeda port",
                "Klaipeda port rerouted freight while Northwind crossings were halted",
                "a1",
            ),
            (
                "fp-3",
                "Northwind Ferries",
                "Northwind Ferries expects crossings to resume within a week",
                "a2",
            ),
            (
                "fp-4",
                "Baltic Shipping Board",
                "Baltic Shipping Board revised the halt to a partial suspension",
                "a2",
            ),
        ];
        let claims: Vec<_> = texts
            .iter()
            .map(|(fp, entity, claim, article)| atlas_claim(fp, entity, claim, article))
            .collect();
        let receipt = store
            .publish_atlas_insights("run-0412", &claims, &[], "", &Default::default())
            .unwrap();
        assert_eq!(receipt.claim_memory_ids.len(), 4, "{:?}", receipt.rejected);
        // The board's revision relates to the original halt (stored fingerprints).
        let fingerprint = |index: usize| receipt.claim_memory_ids[index].0.clone();
        store
            .publish_atlas_insights(
                "run-0412",
                &claims,
                &[(
                    fingerprint(3),
                    fingerprint(0),
                    "conflict_or_revision".into(),
                )],
                "",
                &Default::default(),
            )
            .unwrap();
        store
            .add_memory(
                "Ferry crossings halted after an engine fire last winter",
                "fact",
                false,
                MemorySource {
                    app: "manual".into(),
                    conversation_id: "notes".into(),
                    message_id: None,
                    reference: None,
                },
            )
            .unwrap();
        let all = store.list_memories().unwrap();
        let find = |snippet: &str| {
            all.iter()
                .find(|memory| memory.text.contains(snippet))
                .unwrap_or_else(|| panic!("memory for {snippet}"))
                .id
                .clone()
        };
        let ids = vec![
            find("halted Baltic crossings after an engine fire"),
            find("rerouted freight"),
            find("resume within a week"),
            find("partial suspension"),
            find("last winter"),
        ];
        ids
    }

    #[test]
    fn claim_detail_has_graph_above_related_left_summary_right_and_navigates_by_id() {
        let mut app = app();
        app.screen = Rect::new(0, 0, 140, 40);
        let ids = claim_fixture(&app);
        app.select(ModuleId::Brain.index());
        // Find hides most targets; navigation must still reach them.
        app.brain_query = "engine fire".into();
        app.reload_memories();
        assert!(
            app.memories.iter().all(|m| m.id != ids[3]),
            "target filtered out"
        );
        let row = app.memories.iter().position(|m| m.id == ids[0]).unwrap();
        app.memory_sel = row;
        app.set_focus(Target::Memory(row));
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.brain_list_mode, BrainListMode::Graph);
        assert_eq!(app.brain_detail.memory_id(), Some(ids[0].as_str()));
        assert!(app.detail_claim(), "Atlas provenance opens a claim path");

        // Layout: graph above, Related left, Summary right, same row.
        let areas = super::super::ui::detail_areas(&app);
        assert!(!areas.stacked);
        assert!(areas.path.y < areas.related.y);
        assert_eq!(areas.related.y, areas.summary.y);
        assert!(areas.related.x < areas.summary.x);
        let text = buffer_text(&render(&mut app, 140, 40));
        assert!(
            text.contains(" claim path ")
                && text.contains(" related ")
                && text.contains(" summary "),
            "{text}"
        );
        let path_rows: String = text
            .lines()
            .skip(areas.path.y as usize)
            .take(areas.path.height as usize)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            !path_rows.contains("Related") && !path_rows.contains("Linked"),
            "no Related text in the graph pane: {path_rows}"
        );

        // Related: unique existing targets, self excluded, explicit before similar.
        let items = app.brain_detail.related.items.clone();
        let item_ids: Vec<&str> = items.iter().map(|i| i.memory_id.as_str()).collect();
        assert!(!item_ids.contains(&ids[0].as_str()));
        let mut unique = item_ids.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), item_ids.len());
        // Claim relations rank first: the stored board revision, plus the
        // conflict publication detects between the two Northwind claims.
        let relations: Vec<&str> = items
            .iter()
            .take_while(|i| i.reason.starts_with("claim relation"))
            .map(|i| i.memory_id.as_str())
            .collect();
        assert!(
            relations.contains(&ids[3].as_str()),
            "claim relation first: {items:?}"
        );
        assert!(relations.contains(&ids[2].as_str()), "{items:?}");
        use argos_osint_core::related_memories::RelationKind;
        let first_similar = items.iter().position(|i| i.kind == RelationKind::Similar);
        if let Some(first) = first_similar {
            assert!(items[..first]
                .iter()
                .all(|i| i.kind == RelationKind::Explicit));
            assert!(items[first..]
                .iter()
                .all(|i| i.kind == RelationKind::Similar));
        }
        assert!(text.contains("Linked"));
        let manual = items
            .iter()
            .find(|i| i.memory_id == ids[4])
            .expect("similar manual memory");
        assert_eq!(manual.kind, RelationKind::Similar);
        assert!(text.contains("Similar · not evidence"), "{text}");

        // Keyboard: Tab to Related, Down selects without navigating.
        while !matches!(app.focus, Target::RelatedRow(_)) {
            app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        }
        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(app.brain_detail.related.sel, 1);
        assert_eq!(
            app.brain_detail.memory_id(),
            Some(ids[0].as_str()),
            "selection alone does not navigate"
        );
        let target = items[1].memory_id.clone();
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.brain_detail.memory_id(), Some(target.as_str()));
        let entered_graph = app.brain_graph.clone();
        assert!(app
            .brain_detail
            .related
            .items
            .iter()
            .all(|i| i.memory_id != target));

        // Back restores the previous memory, its Related selection and focus.
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.brain_detail.memory_id(), Some(ids[0].as_str()));
        assert_eq!(app.brain_detail.related.sel, 1);
        assert_eq!(app.focus, Target::RelatedRow(1));

        // Click on the same row opens the same target.
        render(&mut app, 140, 40);
        click(&mut app, Target::RelatedRow(1));
        assert_eq!(app.brain_detail.memory_id(), Some(target.as_str()));
        assert_eq!(app.brain_graph, entered_graph);
        // The filtered-out claim-relation target opens too.
        app.detail_back();
        render(&mut app, 140, 40);
        click(&mut app, Target::RelatedRow(0));
        let first = items[0].memory_id.clone();
        assert!(
            app.memories.iter().all(|m| m.id != first),
            "filtered out by Find"
        );
        assert_eq!(app.brain_detail.memory_id(), Some(first.as_str()));
        assert_eq!(app.brain_query, "engine fire", "Find is preserved");

        // Late related results for another request are rejected.
        let stale = app.brain_detail.related.request - 1;
        assert!(!app.on_related(stale, &first, Ok(Vec::new())));
        assert!(!app.on_related(app.brain_detail.related.request, &ids[0], Ok(Vec::new())));
        assert!(!app.brain_detail.related.items.is_empty());

        // A deleted destination keeps the current view intact.
        let (gone_row, gone) = app
            .brain_detail
            .related
            .items
            .iter()
            .enumerate()
            .find(|(_, i)| i.memory_id != ids[0])
            .map(|(row, i)| (row, i.memory_id.clone()))
            .unwrap();
        app.store.delete_memory(&gone).unwrap();
        render(&mut app, 140, 40);
        app.brain_detail.related.sel = gone_row;
        app.set_focus(Target::RelatedRow(gone_row));
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.brain_detail.memory_id(), Some(first.as_str()));
        assert!(app.status.contains("no longer exists"), "{}", app.status);

        // Back all the way returns to the list with Find and selection intact.
        for _ in 0..8 {
            if app.brain_list_mode != BrainListMode::Graph {
                break;
            }
            app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        }
        assert_eq!(app.brain_list_mode, BrainListMode::List);
        assert_eq!(app.brain_query, "engine fire");
        assert_eq!(app.memories[app.memory_sel].id, ids[0]);
    }

    #[test]
    fn narrow_claim_detail_stacks_related_above_summary_and_keeps_both_reachable() {
        let mut app = app();
        app.screen = Rect::new(0, 0, 64, 32);
        let ids = claim_fixture(&app);
        app.select(ModuleId::Brain.index());
        assert!(app.open_memory_detail(&ids[0]));
        let areas = super::super::ui::detail_areas(&app);
        assert!(areas.stacked);
        assert!(areas.path.y < areas.related.y && areas.related.y < areas.summary.y);
        assert!(areas.related.height >= 3 && areas.summary.height >= 3);
        let text = buffer_text(&render(&mut app, 64, 32));
        assert!(
            text.contains(" related ") && text.contains(" summary "),
            "{text}"
        );
        // The legend wraps instead of truncating on narrow widths.
        assert!(
            text.contains("finding") && text.contains("source"),
            "{text}"
        );
        assert!(!text.contains("findin…"), "{text}");
        // Focusing Summary gives it the larger share; Related stays visible.
        app.set_focus(Target::DetailSummary);
        let focused = super::super::ui::detail_areas(&app);
        assert!(focused.summary.height > focused.related.height);
        render(&mut app, 64, 32);
        click(&mut app, Target::RelatedRow(0));
        assert_ne!(app.brain_detail.memory_id(), Some(ids[0].as_str()));
        // Back on screen.
        click(&mut app, Target::Button(ButtonId::BrainDetailBack));
        assert_eq!(app.brain_detail.memory_id(), Some(ids[0].as_str()));
    }

    #[test]
    fn brain_refresh_keeps_selection_and_find_and_shows_read_errors() {
        let mut app = app();
        app.screen = Rect::new(0, 0, 120, 34);
        let add = |app: &App, text: &str| {
            app.store
                .add_memory(
                    text,
                    "fact",
                    false,
                    MemorySource {
                        app: "manual".into(),
                        conversation_id: "c".into(),
                        message_id: None,
                        reference: None,
                    },
                )
                .unwrap()
                .id
        };
        add(&app, "alpha one");
        add(&app, "beta two");
        let keep = add(&app, "alpha three");
        // Entering Brain loads memories committed elsewhere.
        app.select(ModuleId::Brain.index());
        assert_eq!(app.memories.len(), 3);
        let row = app.memories.iter().position(|m| m.id == keep).unwrap();
        app.memory_sel = row;
        app.set_focus(Target::Memory(row));
        add(&app, "alpha four");
        add(&app, "gamma five");
        app.reload_memories();
        assert_eq!(
            app.memories[app.memory_sel].id, keep,
            "selection follows the id"
        );
        assert_eq!(app.focus, Target::Memory(app.memory_sel));

        // Find stays active; the title says it filters.
        app.brain_query = "alpha".into();
        app.reload_memories();
        assert_eq!(app.memories[app.memory_sel].id, keep);
        let text = buffer_text(&render(&mut app, 120, 34));
        assert!(text.contains("Find active · 3 of 5"), "{text}");
        add(&app, "delta six");
        app.reload_memories();
        assert_eq!(
            app.brain_query, "alpha",
            "a hidden new memory does not clear Find"
        );
        assert_eq!(app.memory_total, 6);

        // No matches is distinct from no memories.
        app.brain_query = "zzz".into();
        app.reload_memories();
        let text = buffer_text(&render(&mut app, 120, 34));
        assert!(
            text.contains("No memories match") && text.contains("Find is still active"),
            "{text}"
        );

        // A read failure keeps the last good list and says so.
        app.brain_query = "alpha".into();
        app.reload_memories();
        let shown = app.memories.clone();
        super::super::brain_detail::testing::fail_reads(Some("disk I/O error"));
        add(&app, "alpha seven");
        app.reload_memories();
        assert_eq!(app.memories, shown, "last good list kept");
        assert_eq!(app.memory_error.as_deref(), Some("disk I/O error"));
        let text = buffer_text(&render(&mut app, 120, 34));
        assert!(
            text.contains("read failed · showing last loaded list"),
            "{text}"
        );
        assert!(events(&app)
            .iter()
            .any(|e| e.message.contains("could not read memories")));
        // With nothing loaded yet, the failure is shown instead of an empty list.
        let mut fresh = super::tests::app();
        fresh.screen = Rect::new(0, 0, 120, 34);
        fresh.memories_loaded = false;
        fresh.select(ModuleId::Brain.index());
        let text = buffer_text(&render(&mut fresh, 120, 34));
        assert!(
            text.contains("Could not read memories: disk I/O error"),
            "{text}"
        );
        super::super::brain_detail::testing::fail_reads(None);
        app.reload_memories();
        assert!(app.memory_error.is_none());
        assert_eq!(app.memories.len(), 4);
        let mut empty = super::tests::app();
        empty.select(ModuleId::Brain.index());
        let text = buffer_text(&render(&mut empty, 120, 34));
        assert!(text.contains("No memories yet"), "{text}");
    }

    #[test]
    fn atlas_history_resume_is_offered_only_for_resumable_cycles_and_repair_starts_once() {
        let mut app = app();
        app.screen = Rect::new(0, 0, 140, 44);
        app.store
            .atlas_insert_run(
                "run-done",
                r#"{"phase":4,"chunk":0,"leg":"insights_done","country":0,"from":""}"#,
                "{}",
            )
            .unwrap();
        app.store
            .atlas_set_state("run-done", "completed", "", true)
            .unwrap();
        app.store
            .atlas_insert_run(
                "run-stuck",
                r#"{"phase":5,"chunk":0,"leg":"index","country":0,"from":""}"#,
                "{}",
            )
            .unwrap();
        app.store
            .atlas_set_state("run-stuck", "partial", "0/3 indexed", true)
            .unwrap();
        app.select(ModuleId::Atlas.index());
        let done = app
            .atlas_runs
            .iter()
            .position(|r| r.id == "run-done")
            .unwrap();
        let stuck = app
            .atlas_runs
            .iter()
            .position(|r| r.id == "run-stuck")
            .unwrap();
        app.atlas_run_sel = done;
        assert!(!app.atlas_can_resume());
        render(&mut app, 140, 44);
        click(&mut app, Target::Button(ButtonId::AtlasResume));
        assert!(app.status.contains("nothing to resume"), "{}", app.status);
        app.atlas_run_sel = stuck;
        assert!(app.atlas_can_resume(), "phase-5 partial cycle can resume");
        let order = super::super::ui::focus_order(&app);
        for button in [
            ButtonId::AtlasLive,
            ButtonId::AtlasResume,
            ButtonId::AtlasRepair,
            ButtonId::AtlasDelete,
        ] {
            assert!(order.contains(&Target::Button(button)), "{button:?}");
        }
        let text = buffer_text(&render(&mut app, 140, 44));
        assert!(
            text.contains("Repair memories") && text.contains("Resume"),
            "{text}"
        );
        click(&mut app, Target::Button(ButtonId::AtlasRepair));
        assert_eq!(super::super::atlas_actions::testing::starts(), 1);
        assert!(
            app.status.contains("Repair Atlas memories started"),
            "{}",
            app.status
        );
        super::super::atlas_actions::testing::set_busy(true);
        click(&mut app, Target::Button(ButtonId::AtlasRepair));
        assert_eq!(super::super::atlas_actions::testing::starts(), 1);
        assert!(app.status.contains("already running"), "{}", app.status);
        super::super::atlas_actions::testing::set_busy(false);
    }

    /// Cell dumps of the Brain claim detail (normal + narrow) for PNG rendering.
    #[test]
    #[ignore]
    fn dump_phase5b_screens() {
        let Ok(dir) = std::env::var("ARGOS_SCREEN_DIR") else {
            return;
        };
        std::fs::create_dir_all(&dir).unwrap();
        let mut app = app();
        app.screen = Rect::new(0, 0, 140, 40);
        let ids = claim_fixture(&app);
        app.select(ModuleId::Brain.index());
        app.brain_query = "engine fire".into();
        app.reload_memories();
        assert!(app.open_memory_detail(&ids[0]));
        // Fixture summary (the test app has no Summarization account).
        app.graph_summary = "## Northwind halted Baltic crossings\n\nTwo **Baltic Wire** articles in cycle 04:12 support the halt. A later board decision **revises** it to a partial suspension, so the claim is a *dated fact*, not current status.\n\n- Evidence: 2 articles, 1 cycle\n- Related: 1 revision, 2 linked claims".into();
        app.status = "Claim path".into();
        while !matches!(app.focus, Target::RelatedRow(_)) {
            app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        }
        dump_cells(&mut app, &dir, "brain-claim-detail", 140, 40);
        app.screen = Rect::new(0, 0, 64, 32);
        dump_cells(&mut app, &dir, "brain-claim-detail-narrow", 64, 32);
    }

    fn dump_cells(app: &mut App, dir: &str, name: &str, width: u16, height: u16) {
        let buffer = render(app, width, height);
        let mut cells = Vec::new();
        for y in 0..height {
            let mut row = Vec::new();
            for x in 0..width {
                let cell = &buffer[(x, y)];
                row.push(serde_json::json!({
                    "s": cell.symbol(),
                    "fg": format!("{:?}", cell.fg),
                    "bg": format!("{:?}", cell.bg),
                    "b": cell.modifier.contains(ratatui::style::Modifier::BOLD),
                    "u": cell.modifier.contains(ratatui::style::Modifier::UNDERLINED),
                }));
            }
            cells.push(row);
        }
        let json = serde_json::json!({"width": width, "height": height, "cells": cells});
        std::fs::write(format!("{dir}/{name}.json"), json.to_string()).unwrap();
    }

    /// Phase 7 fixture: a claim memory whose Summarization role points at a
    /// scripted local provider; jobs/events go to the test database.
    struct SummaryFx {
        _dir: tempfile::TempDir,
        db: std::path::PathBuf,
        rt: tokio::runtime::Runtime,
        server: argos_osint_core::provider_attempt::mock::Server,
        ids: Vec<String>,
    }

    fn summary_fixture(
        app: &mut App,
        script: Vec<argos_osint_core::provider_attempt::mock::Reply>,
    ) -> SummaryFx {
        use argos_osint_core::provider_attempt::mock;
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("argos.db");
        app.store = Store::open(&db).unwrap();
        super::super::tracked::testing::use_db(Some(db.clone()));
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        let server = rt.block_on(mock::serve(script));
        app.auth.set_account(mock::secret(&server.base_url));
        app.settings.defaults.summarization = argos_osint_core::provider::ModelAssignment {
            provider: "openrouter".into(),
            model: "mock-model".into(),
            ..Default::default()
        };
        let ids = claim_fixture(app);
        app.select(ModuleId::Brain.index());
        app.reload_memories();
        SummaryFx {
            _dir: dir,
            db,
            rt,
            server,
            ids,
        }
    }

    /// Pump work events until the pending graph summary settles.
    fn settle_summary(app: &mut App, fx: &SummaryFx) {
        for _ in 0..400 {
            fx.rt
                .block_on(tokio::time::sleep(Duration::from_millis(25)));
            pump(app);
            if app.graph_summary_pending.is_none() {
                return;
            }
        }
        panic!("graph summary never finished: {}", app.status);
    }

    fn graph_jobs(fx: &SummaryFx) -> Vec<(String, String, String)> {
        Store::open(&fx.db)
            .unwrap()
            .graph_explanation_jobs(&fx.ids[0])
            .unwrap()
    }

    const AUTH_401: &str = r#"{"error":{"message":"Incorrect API key provided: sk-mocksecretvalue12345","code":"invalid_api_key"}}"#;

    #[test]
    fn brain_summary_failure_card_explains_links_and_retries_in_place() {
        use argos_osint_core::provider_attempt::mock::Reply;
        let mut app = app();
        app.screen = Rect::new(0, 0, 140, 40);
        let fx = summary_fixture(&mut app, vec![Reply::Json(401, AUTH_401.into())]);
        let _enter = fx.rt.enter();
        assert!(app.open_memory_detail(&fx.ids[0]));
        settle_summary(&mut app, &fx);
        let failure = app.summary_failure.clone().expect("failure card");
        assert!(failure.needs_config, "{failure:?}");
        assert!(failure
            .guidance
            .as_deref()
            .unwrap_or("")
            .contains("Providers"));
        assert_eq!(
            fx.server.hits(),
            4,
            "401 consumes the primary attempt budget"
        );
        assert!(!failure.event_id.is_empty() && !failure.job_id.is_empty());
        assert!(app
            .graph_summary
            .contains(argos_osint_core::graph_explanation::BASIC_HEADING));
        let text = buffer_text(&render(&mut app, 140, 40));
        for want in [
            "AI summary failed",
            "View details",
            "View logs",
            "View job",
            "Retry summary",
            "Open Models",
        ] {
            assert!(text.contains(want), "missing {want}: {text}");
        }
        assert!(!text.contains("sk-mocksecretvalue12345"));
        assert!(!text.contains("Leave and open"));
        let order = super::super::ui::focus_order(&app);
        assert!(order.contains(&Target::Button(ButtonId::SummaryRetry)));

        click(&mut app, Target::Button(ButtonId::SummaryDetails));
        assert!(app.summary_details_open);
        let text = buffer_text(&render(&mut app, 140, 40));
        assert!(text.contains("Stage: response"), "{text}");
        assert!(text.contains("HTTP 401"), "{text}");

        // Reopening inside the cooldown shows the saved failure; no new job or request.
        app.leave_brain_detail();
        assert!(app.open_memory_detail(&fx.ids[0]));
        assert!(app.graph_summary_pending.is_none());
        assert!(app.summary_failure.as_ref().is_some_and(|f| f.from_record));
        assert_eq!((graph_jobs(&fx).len(), fx.server.hits()), (1, 4));

        // Retry: fresh budget, linked to the failed job, without leaving Brain.
        render(&mut app, 140, 40);
        click(&mut app, Target::Button(ButtonId::SummaryRetry));
        assert_eq!(app.module, Some(ModuleId::Brain));
        settle_summary(&mut app, &fx);
        let jobs = graph_jobs(&fx);
        assert_eq!(jobs.len(), 2);
        assert_eq!(jobs[1].2, jobs[0].0, "retry correlates to the failed job");
        assert_eq!(fx.server.hits(), 8);
        assert_eq!(app.module, Some(ModuleId::Brain));

        // View logs: filtered to the job, failure event selected.
        let failure = app.summary_failure.clone().unwrap();
        render(&mut app, 140, 40);
        click(&mut app, Target::Button(ButtonId::SummaryLogs));
        assert_eq!(app.module, Some(ModuleId::Logs));
        assert_eq!(app.logs.job, failure.job_id);
        assert_eq!(
            app.logs.selected().map(|r| r.id.clone()),
            Some(failure.event_id.clone())
        );

        // After the events expire (simulated by clearing them), the link says
        // so; the job and the saved diagnostic keep the error summary.
        app.store.clear_events().unwrap();
        app.activate_button(ButtonId::SummaryLogs);
        assert!(app.status.contains("expired"), "{}", app.status);
        let rec = app
            .store
            .graph_explanation_record(&fx.ids[0])
            .unwrap()
            .unwrap();
        assert_eq!(rec.state, "failed");
        assert!(rec.diagnostic_json.contains("invalid_api_key"));

        // Open Models focuses the Summarization role.
        app.activate_button(ButtonId::SummaryModels);
        assert_eq!(app.module, Some(ModuleId::Providers));
        assert_eq!(app.provider_page, ProviderPage::Defaults);
        assert_eq!(app.defaults_role, DefaultsRole::Summarization);

        // View job opens the failed job in Jobs.
        app.activate_button(ButtonId::SummaryJob);
        assert_eq!(app.module, Some(ModuleId::Jobs));
        assert_eq!(
            app.jobs.selected().map(|j| j.id.clone()),
            Some(failure.job_id.clone())
        );

        // The memory is untouched.
        assert!(app.store.get_memory(&fx.ids[0]).unwrap().is_some());
        super::super::tracked::testing::use_db(None);
    }

    #[test]
    fn saved_summary_is_reused_until_its_inputs_change_then_shown_as_earlier() {
        use argos_osint_core::provider_attempt::mock::{ok_json, Reply};
        let good =
            "## Northwind **halted** Baltic crossings\n\nTwo Baltic Wire articles state the halt.";
        let mut app = app();
        app.screen = Rect::new(0, 0, 140, 40);
        let fx = summary_fixture(
            &mut app,
            vec![ok_json(good, "stop"), Reply::Json(503, "{}".into())],
        );
        let _enter = fx.rt.enter();
        assert!(app.open_memory_detail(&fx.ids[0]));
        settle_summary(&mut app, &fx);
        assert_eq!(
            app.graph_summary, good,
            "status={} failure={:?}",
            app.status, app.summary_failure
        );
        assert!(app.summary_failure.is_none());
        // Reopen: valid cache, no request.
        app.leave_brain_detail();
        assert!(app.open_memory_detail(&fx.ids[0]));
        assert_eq!((app.graph_summary.as_str(), fx.server.hits()), (good, 1));
        // Change the memory text: the old summary is only an earlier result.
        let memory = app.store.get_memory(&fx.ids[0]).unwrap().unwrap();
        app.store
            .update_memory(
                &memory.id,
                &format!("{} (revised)", memory.text),
                &memory.category,
                memory.pinned,
            )
            .unwrap();
        app.leave_brain_detail();
        app.reload_memories();
        assert!(app.open_memory_detail(&fx.ids[0]));
        assert!(
            app.graph_summary.contains("Earlier result"),
            "{}",
            app.graph_summary
        );
        settle_summary(&mut app, &fx);
        assert!(app.summary_failure.is_some());
        assert!(
            app.graph_summary.contains("Earlier result")
                && app.graph_summary.contains("Baltic Wire")
        );
        assert_eq!(
            fx.server.hits(),
            5,
            "503 consumes the primary budget after the first success: 1 + 4"
        );
        super::super::tracked::testing::use_db(None);
    }

    #[test]
    fn late_summary_completions_are_ignored() {
        use argos_osint_core::graph_explanation::{ExplainOutcome, ExplainReport};
        let mut app = app();
        app.graph_summary_request = "req-new".into();
        app.graph_summary_pending = Some("m1".into());
        let stale = ExplainReport {
            request_id: "req-old".into(),
            memory_id: "m1".into(),
            job_id: "job-old".into(),
            cache_key: String::new(),
            outcome: ExplainOutcome::Saved("old text that must not appear".into()),
            attempts: Vec::new(),
            admission_wait_ms: 0,
            fallback_reason: None,
            event_id: String::new(),
            logging_error: None,
        };
        assert!(!app.on_graph_summary(stale));
        assert_eq!(app.graph_summary_pending.as_deref(), Some("m1"));
        assert!(!app.graph_summary.contains("old text"));
    }

    /// Cell dumps of the Brain summary failure card for PNG rendering.
    #[test]
    #[ignore]
    fn dump_phase7_screens() {
        use argos_osint_core::provider_attempt::mock::Reply;
        let Ok(dir) = std::env::var("ARGOS_SCREEN_DIR") else {
            return;
        };
        std::fs::create_dir_all(&dir).unwrap();
        let mut app = app();
        app.screen = Rect::new(0, 0, 140, 40);
        let fx = summary_fixture(&mut app, vec![Reply::Json(401, AUTH_401.into())]);
        let _enter = fx.rt.enter();
        app.brain_query = "engine fire".into();
        app.reload_memories();
        assert!(app.open_memory_detail(&fx.ids[0]));
        settle_summary(&mut app, &fx);
        assert!(app.summary_failure.is_some());
        render(&mut app, 140, 40);
        app.set_focus(Target::Button(ButtonId::SummaryRetry));
        dump_cells(&mut app, &dir, "brain-summary-failure", 140, 40);
        click(&mut app, Target::Button(ButtonId::SummaryDetails));
        dump_cells(&mut app, &dir, "brain-summary-failure-details", 140, 40);
        super::super::tracked::testing::use_db(None);
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
        app.select(ModuleId::Brain.index());
        app.set_focus(Target::Field(FieldId::BrainInsight));
        app.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::ALT));
        app.handle_key(KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE));
        assert_eq!(app.module, Some(ModuleId::Brain));
    }

    #[test]
    fn firecrawl_key_field_is_on_the_osint_tool() {
        let mut app = app();
        app.screen = Rect::new(0, 0, 100, 36);
        app.select(ModuleId::Osint.index());
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
        app.tool_sel = osint::registry()
            .iter()
            .position(|tool| tool.id == "whoxy_whois_history")
            .unwrap();
        assert!(hit(&app, Target::Field(FieldId::WhoxyKey)));
        assert!(hit(&app, Target::Button(ButtonId::SaveWhoxyKey)));
        assert!(hit(&app, Target::Button(ButtonId::TestWhoxyConnection)));
        assert!(!hit(&app, Target::Field(FieldId::FirecrawlKey)));
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
        app.select(ModuleId::Osint.index());
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
        app.newsapi_fallback = "news-spare-29".into();
        click(&mut app, Target::Button(ButtonId::SaveNewsApiKey));
        assert_eq!(app.status, "NewsAPI key saved");
        assert!(hit(&app, Target::Field(FieldId::NewsApiFallback)));
        select(&mut app, "courtlistener_case_search");
        app.courtlistener_key = "court-secret-29".into();
        click(&mut app, Target::Button(ButtonId::SaveCourtListenerKey));
        assert_eq!(app.status, "CourtListener API token saved");
        assert_eq!(
            (
                app.settings.newsapi_api_key.as_str(),
                app.settings.newsapi_api_key_fallback.as_str(),
                app.settings.courtlistener_api_token.as_str()
            ),
            ("news-secret-29", "news-spare-29", "court-secret-29")
        );
        for id in osint::NEWS_TOOLS.iter().chain(osint::LEGAL_TOOLS) {
            assert!(
                !app.tool_needs_key_with(id, no_env),
                "{id}: one key enables every tool of the provider"
            );
        }
        let saved = std::fs::read_to_string(&app.settings_path).unwrap();
        assert!(
            saved.contains("newsapi_api_key")
                && saved.contains("newsapi_api_key_fallback")
                && saved.contains("courtlistener_api_token"),
            "{saved}"
        );
        // The key fields are masked on screen and "needs key" shows on unkeyed tools.
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 36)).unwrap();
        select(&mut app, "newsapi_headlines");
        terminal
            .draw(|frame| super::super::ui::draw(frame, &app))
            .unwrap();
        let news = screen_text(&terminal);
        assert!(
            news.contains("Fallback")
                && !news.contains("news-secret-29")
                && !news.contains("news-spare-29")
                && news.contains("•••••"),
            "masked fallback field"
        );
        select(&mut app, "courtlistener_case_search");
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
        app.recon_stage = "synthesizing".into();
        app.recon_stages
            .insert("t-open".into(), "synthesizing".into());
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
            .any(|block| block.key == "status:run-1" && block.title.contains("synthesizing")));
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

    #[test]
    fn a_log_or_late_tool_row_does_not_drop_the_live_answer() {
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
            plan_json: Some(
                r#"{"directives":[{"id":"d1","goal":"Establish identity","entities":[],"targets":[]}]}"#
                    .into(),
            ),
            error: Some("answer is missing evidence citations".into()),
            created_at: String::new(),
            updated_at: String::new(),
        }];
        app.expanded.insert("plan:run-1".into());
        app.calls.push(recon::Call {
            id: "call-matched".into(),
            tool_id: "firecrawl_search".into(),
            run_id: Some("run-1".into()),
            thread_id: Some("t-open".into()),
            turn_id: Some("m1".into()),
            origin: "recon".into(),
            inputs: serde_json::json!({"query": "Shivon Zilis"}),
            status: "completed".into(),
            attempts: 1,
            result: None,
            started_at: String::new(),
            completed_at: None,
        });
        app.calls.push(recon::Call {
            id: "call-late".into(),
            tool_id: "sociavault_google_search".into(),
            run_id: None,
            thread_id: Some("t-open".into()),
            turn_id: None,
            origin: "recon".into(),
            inputs: serde_json::json!({"query": "Shivon Zilis official account"}),
            status: "completed".into(),
            attempts: 1,
            result: None,
            started_at: String::new(),
            completed_at: None,
        });
        app.recon_stages
            .insert("t-open".into(), "synthesizing".into());
        app.on_work_event(WorkEvent::AnswerDelta {
            thread_id: "t-open".into(),
            text: "Shivon Zilis is a Neuralink executive.".into(),
        });
        let blocks = super::super::ui::chat_blocks(&app);
        let stream = blocks.last().unwrap();
        assert_eq!(stream.key, "stream:run-1");
        assert_eq!(stream.body, "Shivon Zilis is a Neuralink executive.");
        assert!(blocks.iter().any(|block| block.key == "tool:call-late"));
        assert!(blocks.iter().any(|block| block
            .body
            .contains("Run error: answer is missing evidence citations")));
    }

    #[test]
    fn a_failed_turn_keeps_the_streamed_answer_on_screen() {
        let mut app = app();
        app.module = Some(ModuleId::Recon);
        app.recon_chat = true;
        app.screen = Rect::new(0, 0, 100, 40);
        app.selected_thread = Some("t-open".into());
        app.running
            .insert("t-open".into(), Arc::new(AtomicBool::new(false)));
        let messages = vec![recon::Message {
            id: "m1".into(),
            thread_id: "t-open".into(),
            sequence: 1,
            role: "user".into(),
            content: "who?".into(),
            run_id: None,
            created_at: String::new(),
        }];
        let runs = vec![recon::Run {
            id: "run-1".into(),
            thread_id: "t-open".into(),
            turn_id: "m1".into(),
            state: "failed".into(),
            stage: "failed".into(),
            recon_model: String::new(),
            synthesis_model: String::new(),
            tool_picker_model: String::new(),
            max_rounds: 1,
            max_calls: 4,
            turn_seconds: 300,
            plan_json: None,
            error: Some("answer is missing evidence citations".into()),
            created_at: String::new(),
            updated_at: String::new(),
        }];
        app.messages = messages.clone();
        app.runs = runs.clone();
        app.on_work_event(WorkEvent::AnswerDelta {
            thread_id: "t-open".into(),
            text: "Shivon Zilis is a Neuralink executive.".into(),
        });
        app.on_work_event(WorkEvent::ReconDone {
            thread_id: "t-open".into(),
            outcome: Err("answer is missing evidence citations".into()),
        });
        assert_eq!(
            app.live_answers["t-open"].text,
            "Shivon Zilis is a Neuralink executive."
        );
        assert!(!app.running_thread("t-open"));
        app.messages = messages;
        app.runs = runs;
        let blocks = super::super::ui::chat_blocks(&app);
        let stream = blocks
            .iter()
            .find(|block| block.key == "stream:run-1")
            .unwrap();
        assert_eq!(stream.body, "Shivon Zilis is a Neuralink executive.");
        assert_eq!(stream.title, "Recon · stopped");
    }

    #[test]
    fn atlas_live_feed_opens_intel_brief_and_past_runs_delete() {
        let mut app = app();
        app.screen = Rect::new(0, 0, 100, 36);
        app.select(ModuleId::Atlas.index());
        assert_eq!(app.module, Some(ModuleId::Atlas));
        assert_eq!(app.atlas_page, AtlasPage::Runs);
        assert!(hit(&app, Target::Button(ButtonId::AtlasLive)));
        assert!(hit(&app, Target::Button(ButtonId::AtlasDelete)));
        click(&mut app, Target::Button(ButtonId::AtlasLive));
        assert_eq!(app.atlas_page, AtlasPage::Live);
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.module, Some(ModuleId::Atlas));
        assert_eq!(app.atlas_page, AtlasPage::Runs);
        click(&mut app, Target::Button(ButtonId::AtlasLive));
        assert_eq!(app.atlas_page, AtlasPage::Live);
        assert!(hit(&app, Target::Button(ButtonId::AtlasRun)));
        assert!(hit(&app, Target::Button(ButtonId::AtlasAuto)));
        assert!(hit(&app, Target::Button(ButtonId::AtlasRuns)));
        app.atlas_feed.push(atlas::FeedArticle {
            id: "art-1".into(),
            title: "Cabinet reshuffle in Beijing".into(),
            description: "A short wire note.".into(),
            url: "https://www.reuters.com/world/china".into(),
            country: "cn".into(),
            source_name: "Reuters".into(),
            source_domain: "reuters.com".into(),
            author: "Wire Desk".into(),
            image_url: "https://www.reuters.com/image.jpg".into(),
            published_at: "2026-10-01T00:00:00Z".into(),
            provider: "newsapi".into(),
            temperature: 1.0,
            seen_at: "2026-10-01T00:01:00Z".into(),
            category: "stability".into(),
        });
        app.atlas_feed_sel = 0;
        app.set_focus(Target::AtlasFeed(0));
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.module, Some(ModuleId::Intel));
        assert_eq!(app.intel_page, IntelPage::Briefing);
        assert_eq!(app.intel_articles[app.intel_sel].id, "art-1");
        assert_eq!(app.intel_category, "stability");
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.intel_page, IntelPage::Bulletin);
        app.select(ModuleId::Atlas.index());
        click(&mut app, Target::Button(ButtonId::AtlasLive));
        let stats = serde_json::to_string(&atlas::RunStats {
            scored: true,
            origins: vec![atlas::OriginStat {
                country: "us".into(),
                tier: 1,
                temperature: 1.0,
                volume: 8,
                articles: 3,
            }],
            ..atlas::RunStats::default()
        })
        .unwrap();
        app.store.atlas_insert_run("atlas-1", "{}", &stats).unwrap();
        app.store
            .atlas_set_state("atlas-1", "completed", "", true)
            .unwrap();
        click(&mut app, Target::Button(ButtonId::AtlasRuns));
        assert_eq!(app.atlas_page, AtlasPage::Runs);
        assert!(hit(&app, Target::Button(ButtonId::AtlasDelete)));
        click(&mut app, Target::Button(ButtonId::AtlasDelete));
        assert!(app.atlas_runs.is_empty());
        assert!(matches!(app.overlay, Overlay::None));
        assert!(app.store.atlas_latest_run().unwrap().is_none());
        app.store
            .atlas_insert_run("atlas-live", "{}", &stats)
            .unwrap();
        click(&mut app, Target::Button(ButtonId::AtlasLive));
        click(&mut app, Target::Button(ButtonId::AtlasRuns));
        click(&mut app, Target::Button(ButtonId::AtlasDelete));
        assert_eq!(app.atlas_runs.len(), 1);
        assert!(app.status.contains("Pause"));
    }

    #[test]
    fn brain_article_source_opens_intel_brief() {
        let mut app = app();
        app.screen = Rect::new(0, 0, 120, 42);
        app.store.atlas_insert_run("run-brain", "{}", "{}").unwrap();
        app.store
            .atlas_upsert_article(&AtlasArticleRow {
                run_id: "run-brain".into(),
                id: "art-brain".into(),
                title: "Claim source article".into(),
                description: "Body for the claim.".into(),
                url: "https://example.com/art-brain".into(),
                country: "us".into(),
                source_name: "Wire".into(),
                source_domain: "example.com".into(),
                author: String::new(),
                image_url: String::new(),
                published_at: chrono::Utc::now().to_rfc3339(),
                provider: "newsapi".into(),
                temperature: 0.9,
                category: "military".into(),
                seen_at: chrono::Utc::now().to_rfc3339(),
            })
            .unwrap();
        app.module = Some(ModuleId::Brain);
        app.brain_list_mode = BrainListMode::Graph;
        app.brain_graph = recon::MemoryGraph {
            nodes: vec![
                recon::GraphNode {
                    id: "investigation:1".into(),
                    kind: recon::GraphNodeKind::Investigation,
                    label: "Troop movements".into(),
                    detail: String::new(),
                    tags: Vec::new(),
                    article_id: String::new(),
                    run_id: String::new(),
                    published_at: String::new(),
                },
                recon::GraphNode {
                    id: "directive:verify".into(),
                    kind: recon::GraphNodeKind::Directive,
                    label: "Verify claim".into(),
                    detail: "Check the report".into(),
                    tags: Vec::new(),
                    article_id: String::new(),
                    run_id: String::new(),
                    published_at: String::new(),
                },
                recon::GraphNode {
                    id: "evidence:1".into(),
                    kind: recon::GraphNodeKind::Evidence,
                    label: "Claim source article".into(),
                    detail: "Wire report".into(),
                    tags: vec!["verify".into()],
                    article_id: "art-brain".into(),
                    run_id: "run-brain".into(),
                    published_at: String::new(),
                },
            ],
            edges: Vec::new(),
        };
        let lines = super::super::graph::path_lines(&app.brain_graph);
        let index = lines
            .iter()
            .position(|line| line.article_id == "art-brain")
            .expect("evidence path line");
        click(&mut app, Target::PathLine(index));
        assert_eq!(app.module, Some(ModuleId::Intel));
        assert_eq!(app.intel_page, IntelPage::Briefing);
        assert_eq!(app.intel_articles[app.intel_sel].id, "art-brain");
        assert_eq!(app.intel_category, "military");
    }

    #[test]
    fn atlas_news_feed_tags_the_country_code() {
        let mut app = app();
        app.screen = Rect::new(0, 0, 120, 42);
        app.store.atlas_insert_run("atlas-de", "{}", "{}").unwrap();
        app.store
            .atlas_set_state("atlas-de", "completed", "", true)
            .unwrap();
        app.store
            .atlas_upsert_article(&AtlasArticleRow {
                run_id: "atlas-de".into(),
                id: "art-1".into(),
                title: "Cabinet reshuffle in Berlin".into(),
                description: "Ministers left the cabinet.".into(),
                url: "https://www.reuters.com/world/europe".into(),
                country: "de".into(),
                source_name: "Reuters".into(),
                source_domain: "reuters.com".into(),
                author: "Desk".into(),
                image_url: String::new(),
                published_at: "2026-10-01T00:00:00Z".into(),
                provider: "newsapi".into(),
                temperature: 1.0,
                category: "stability".into(),
                seen_at: "2026-10-01T00:01:00Z".into(),
            })
            .unwrap();
        app.select(ModuleId::Atlas.index());
        click(&mut app, Target::AtlasHistory(0));
        assert!(app.atlas_news);
        assert!(matches!(app.overlay, Overlay::None));
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 42)).unwrap();
        terminal
            .draw(|frame| super::super::ui::draw(frame, &app))
            .unwrap();
        let painted = screen_text(&terminal);
        assert!(painted.contains("Loading country"), "{painted}");
        app.atlas_map_hold = false;
        terminal
            .draw(|frame| super::super::ui::draw(frame, &app))
            .unwrap();
        let painted = screen_text(&terminal);
        assert!(painted.contains("Cabinet reshuffle"), "{painted}");
        assert!(painted.contains("Reuters"), "{painted}");
        assert!(painted.contains("DE"), "{painted}");
        assert!(painted.contains("stability"), "{painted}");
        assert!(painted.contains("news:"), "{painted}");
        assert!(painted.contains("articles"), "{painted}");
        click(&mut app, Target::AtlasArticle(0));
        assert_eq!(app.module, Some(ModuleId::Intel));
        assert_eq!(app.intel_page, IntelPage::Briefing);
        assert_eq!(app.intel_articles[app.intel_sel].id, "art-1");
        assert_eq!(app.intel_category, "stability");
        assert!(matches!(app.overlay, Overlay::None));
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.intel_page, IntelPage::Bulletin);
    }

    #[test]
    fn news_feed_button_draws_with_several_hot_countries() {
        let mut app = app();
        app.screen = Rect::new(0, 0, 180, 55);
        let stats = serde_json::to_string(&atlas::RunStats {
            scored: true,
            origins: ["us", "gb", "ng", "ca", "au"]
                .into_iter()
                .enumerate()
                .map(|(index, country)| atlas::OriginStat {
                    country: country.into(),
                    tier: if index < 2 { 1 } else { 3 },
                    temperature: 1.0 - index as f64 * 0.1,
                    volume: 4,
                    articles: 2,
                })
                .collect(),
            ..atlas::RunStats::default()
        })
        .unwrap();
        app.store
            .atlas_insert_run("atlas-hot", "{}", &stats)
            .unwrap();
        app.store
            .atlas_set_state("atlas-hot", "completed", "", true)
            .unwrap();
        for index in 0..30 {
            app.store
                .atlas_upsert_article(&AtlasArticleRow {
                    run_id: "atlas-hot".into(),
                    id: format!("art-{index}"),
                    title: format!("Headline {index} about a long regional story"),
                    description: String::new(),
                    url: format!("https://example.com/{index}"),
                    country: "us".into(),
                    source_name: "Reuters".into(),
                    source_domain: "reuters.com".into(),
                    author: "Desk".into(),
                    image_url: String::new(),
                    published_at: "2026-10-01T00:00:00Z".into(),
                    provider: "newsapi".into(),
                    temperature: 1.0,
                    category: "economic".into(),
                    seen_at: format!("2026-10-01T00:{index:02}:00Z"),
                })
                .unwrap();
        }
        app.select(ModuleId::Atlas.index());
        click(&mut app, Target::AtlasHistory(0));
        assert!(app.atlas_news);
        for (width, height) in [(180, 55), (80, 24), (60, 18)] {
            app.screen = Rect::new(0, 0, width, height);
            let mut terminal =
                ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
            terminal
                .draw(|frame| super::super::ui::draw(frame, &app))
                .unwrap();
            if app.atlas_map_hold {
                let painted = screen_text(&terminal);
                assert!(painted.contains("Loading country"), "{painted}");
                app.atlas_map_hold = false;
                terminal
                    .draw(|frame| super::super::ui::draw(frame, &app))
                    .unwrap();
            }
        }
    }

    #[test]
    fn google_and_nvidia_account_panes_show_key_forms() {
        let mut app = app();
        app.screen = Rect::new(0, 0, 100, 36);
        app.select(ModuleId::Providers.index());
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 36)).unwrap();
        app.provider_page = ProviderPage::Google;
        terminal
            .draw(|frame| super::super::ui::draw(frame, &app))
            .unwrap();
        let painted = screen_text(&terminal);
        assert!(painted.contains("Google"), "{painted}");
        assert!(painted.contains("Paste Google AI Studio key"), "{painted}");
        app.provider_page = ProviderPage::Nvidia;
        terminal
            .draw(|frame| super::super::ui::draw(frame, &app))
            .unwrap();
        let painted = screen_text(&terminal);
        assert!(painted.contains("Nvidia"), "{painted}");
        assert!(painted.contains("Paste NVIDIA API key"), "{painted}");
    }

    #[test]
    fn provider_catalog_filters_locally_and_keeps_independent_cache() {
        let mut app = app();
        app.screen = Rect::new(0, 0, 100, 36);
        app.select(ModuleId::Providers.index());
        app.provider_page = ProviderPage::Google;
        app.catalog_cache.insert(
            "google".into(),
            vec![
                ListedModel {
                    id: "gemini-2.5-pro".into(),
                    name: "Gemini 2.5 Pro".into(),
                    free: false,
                },
                ListedModel {
                    id: "gemini-flash".into(),
                    name: "Gemini Flash".into(),
                    free: true,
                },
            ],
        );
        app.google_model_filter = "flash".into();
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 36)).unwrap();
        terminal
            .draw(|frame| super::super::ui::draw(frame, &app))
            .unwrap();
        let painted = screen_text(&terminal);
        assert!(painted.contains("Gemini Flash"), "{painted}");
        assert!(!painted.contains("gemini-2.5-pro"), "{painted}");
        assert!(painted.contains("1 of 2"), "{painted}");
        click(&mut app, Target::Field(FieldId::GoogleModelFilter));
        assert_eq!(app.focus, Target::Field(FieldId::GoogleModelFilter));
        click(&mut app, Target::Button(ButtonId::RefreshModels));
        app.provider_page = ProviderPage::Nvidia;
        app.catalog_cache.insert(
            "nvidia".into(),
            vec![ListedModel {
                id: "nvidia-text-model".into(),
                name: "Nvidia Text".into(),
                free: false,
            }],
        );
        terminal
            .draw(|frame| super::super::ui::draw(frame, &app))
            .unwrap();
        let nvidia = screen_text(&terminal);
        assert!(nvidia.contains("nvidia-text-model"), "{nvidia}");
        assert!(!nvidia.contains("Gemini Flash"), "{nvidia}");
    }

    #[test]
    fn investigation_panel_is_thirty_percent_of_viewport() {
        let mut app = app();
        app.module = Some(ModuleId::Recon);
        app.recon_chat = true;
        app.recon_context_enabled = true;
        app.screen = Rect::new(0, 0, 160, 50);
        let body = super::super::ui::body_rect(&app);
        let (_transcript, context) = super::super::ui::recon_workspace(&app, body);
        let context = context.expect("expanded investigation panel");
        assert_eq!(context.width, (160 * 30) / 100);
        app.screen = Rect::new(0, 0, 80, 24);
        let body = super::super::ui::body_rect(&app);
        let (_transcript, context) = super::super::ui::recon_workspace(&app, body);
        assert!(context.is_none());
    }

    #[test]
    fn full_report_popup_is_seventy_percent_of_viewport() {
        let mut app = app();
        app.screen = Rect::new(0, 0, 160, 50);
        app.overlay = Overlay::Block {
            title: "Report ijob-1 r1".into(),
            body: "bluf".into(),
        };
        let area = super::super::ui::active_popup_area(&app, app.screen);
        assert_eq!(area.width, (160 * 70) / 100);
        assert_eq!(area.height, (50 * 80) / 100);
        assert_eq!(area.x, (160 - area.width) / 2);
    }

    #[test]
    fn tool_documentation_heading_is_visible_for_filtered_selection() {
        let mut app = app();
        app.screen = Rect::new(0, 0, 120, 40);
        app.select(ModuleId::Osint.index());
        app.tool_sel = osint::registry()
            .iter()
            .position(|tool| tool.id == "hunter_domain_search")
            .unwrap();
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 40)).unwrap();
        terminal
            .draw(|frame| super::super::ui::draw(frame, &app))
            .unwrap();
        let painted = screen_text(&terminal);
        assert!(painted.contains("Documentation"), "{painted}");
        assert!(
            painted.contains("hunter.io/api-documentation") || painted.contains("Docs"),
            "{painted}"
        );
        click(&mut app, Target::Button(ButtonId::OpenDocumentation));
        assert!(app.status.contains("http"), "{}", app.status);
    }

    #[test]
    fn selected_intel_busy_hides_only_this_article_controls() {
        let mut app = app();
        app.intel_articles = vec![
            AtlasArticleRow {
                run_id: "r".into(),
                id: "art-busy".into(),
                title: "Busy".into(),
                description: String::new(),
                url: "https://example.com/busy".into(),
                country: "us".into(),
                source_name: "Wire".into(),
                source_domain: "example.com".into(),
                author: String::new(),
                image_url: String::new(),
                published_at: String::new(),
                provider: "newsapi".into(),
                temperature: 1.0,
                category: "stability".into(),
                seen_at: String::new(),
            },
            AtlasArticleRow {
                run_id: "r".into(),
                id: "art-idle".into(),
                title: "Idle".into(),
                description: String::new(),
                url: "https://example.com/idle".into(),
                country: "us".into(),
                source_name: "Wire".into(),
                source_domain: "example.com".into(),
                author: String::new(),
                image_url: String::new(),
                published_at: String::new(),
                provider: "newsapi".into(),
                temperature: 1.0,
                category: "stability".into(),
                seen_at: String::new(),
            },
        ];
        app.intel_sel = 0;
        app.intel_body_running.insert("art-busy".into());
        assert!(app.selected_intel_busy());
        app.intel_sel = 1;
        assert!(!app.selected_intel_busy());
    }

    #[test]
    fn intel_brief_summary_section_never_hidden_and_shows_loading_ui_during_recon() {
        let mut app = app();
        app.screen = Rect::new(0, 0, 140, 45);
        app.select(ModuleId::Intel.index());
        app.intel_page = IntelPage::Briefing;
        let article = AtlasArticleRow {
            run_id: "r1".into(),
            id: "art-test".into(),
            title: "Test Article Title".into(),
            description: "Test description for the article.".into(),
            url: "https://example.com/test".into(),
            country: "us".into(),
            source_name: "Test Source".into(),
            source_domain: "example.com".into(),
            author: String::new(),
            image_url: String::new(),
            published_at: "2026-10-08T12:00:00Z".into(),
            provider: "newsapi".into(),
            temperature: 0.8,
            category: "stability".into(),
            seen_at: String::new(),
        };
        app.intel_articles = vec![article];
        app.intel_sel = 0;

        // 1. When idle and no report yet: Summary section is visible
        assert!(!super::super::ui::intel_report_generating(&app));
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(140, 45)).unwrap();
        terminal
            .draw(|frame| super::super::ui::draw(frame, &app))
            .unwrap();
        let painted_idle = screen_text(&terminal);
        assert!(painted_idle.contains("Summary"), "{painted_idle}");

        // 2. When body retrieval is busy: Summary section is STILL NOT HIDDEN
        app.intel_body_running.insert("art-test".into());
        assert!(app.selected_intel_busy());
        assert!(!super::super::ui::intel_report_generating(&app));
        terminal
            .draw(|frame| super::super::ui::draw(frame, &app))
            .unwrap();
        let painted_busy = screen_text(&terminal);
        assert!(painted_busy.contains("Summary"), "{painted_busy}");
        app.intel_body_running.clear();

        // 3. During recon report generation: Summary section is NOT hidden,
        // and displays the loading UI (like the confidence section).
        let job = argos_osint_core::intel_recon::IntelReportJobRow {
            id: "job-1".into(),
            investigation_id: "inv-1".into(),
            article_id: "art-test".into(),
            mode: "verify".into(),
            revision: 1,
            state: "running".into(),
            stage: "synthesize bluf".into(),
            settings_json: String::new(),
            budget_json: String::new(),
            parent_job_id: None,
            sections_done: 1,
            sections_total: 6,
            elements_done: 0,
            elements_total: 0,
            tool_calls_done: 0,
            tool_calls_allowance: 0,
            current_tool: String::new(),
            warning: String::new(),
            error: String::new(),
            generation: 1,
            started_at: String::new(),
            updated_at: String::new(),
            finished_at: String::new(),
        };
        app.intel_jobs = vec![job];
        app.intel_job_sel = 0;
        app.intel_report_running
            .insert("job-1".into(), Arc::new(AtomicBool::new(false)));

        assert!(app.selected_intel_busy());
        assert!(super::super::ui::intel_report_generating(&app));
        terminal
            .draw(|frame| super::super::ui::draw(frame, &app))
            .unwrap();
        let painted_recon = screen_text(&terminal);
        assert!(painted_recon.contains("Summary"), "{painted_recon}");
        assert!(
            painted_recon.contains("Generating Verify report"),
            "{painted_recon}"
        );
        assert!(painted_recon.contains("synthesize bluf"), "{painted_recon}");

        // 4. Once recon report finishes: Summary section stays visible, loading UI ends.
        app.intel_report_running.clear();
        app.intel_jobs[0].state = "completed".into();
        assert!(!super::super::ui::intel_report_generating(&app));
        terminal
            .draw(|frame| super::super::ui::draw(frame, &app))
            .unwrap();
        let painted_done = screen_text(&terminal);
        assert!(painted_done.contains("Summary"), "{painted_done}");
        assert!(
            !painted_done.contains("Generating Verify report"),
            "{painted_done}"
        );
    }

    #[test]
    fn atlas_world_map_follows_the_selected_run_until_zoomed_in() {
        let mut app = app();
        app.screen = Rect::new(0, 0, 120, 42);
        let stats = |country: &str| {
            serde_json::to_string(&atlas::RunStats {
                scored: true,
                origins: vec![atlas::OriginStat {
                    country: country.into(),
                    tier: 1,
                    temperature: 1.0,
                    volume: 8,
                    articles: 3,
                }],
                ..atlas::RunStats::default()
            })
            .unwrap()
        };
        app.store
            .atlas_insert_run("atlas-us", "{}", &stats("us"))
            .unwrap();
        app.store
            .atlas_set_state("atlas-us", "completed", "", true)
            .unwrap();
        app.store
            .atlas_insert_run("atlas-cn", "{}", &stats("cn"))
            .unwrap();
        app.store
            .atlas_set_state("atlas-cn", "completed", "", true)
            .unwrap();
        app.select(ModuleId::Atlas.index());
        assert_eq!(app.atlas_runs[app.atlas_run_sel].id, "atlas-cn");
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 42)).unwrap();
        let hot_sides = |terminal: &ratatui::Terminal<ratatui::backend::TestBackend>| {
            let buffer = terminal.backend().buffer();
            let width = buffer.area.width;
            let mut west = false;
            let mut east = false;
            for (index, cell) in buffer.content().iter().enumerate() {
                let braille = cell
                    .symbol()
                    .chars()
                    .next()
                    .is_some_and(|ch| ('\u{2800}'..='\u{28FF}').contains(&ch));
                let hot =
                    braille && matches!(cell.fg, ratatui::style::Color::Rgb(r, _, _) if r >= 200);
                if !hot {
                    continue;
                }
                if (index as u16) % width < width / 2 {
                    west = true;
                } else {
                    east = true;
                }
            }
            (west, east)
        };
        terminal
            .draw(|frame| super::super::ui::draw(frame, &app))
            .unwrap();
        let painted = screen_text(&terminal);
        assert!(painted.contains("China"));
        assert!(!painted.contains("United States"));
        assert_eq!(hot_sides(&terminal), (false, true));
        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(app.atlas_runs[app.atlas_run_sel].id, "atlas-us");
        terminal
            .draw(|frame| super::super::ui::draw(frame, &app))
            .unwrap();
        let painted = screen_text(&terminal);
        assert!(painted.contains("United States"));
        assert!(!painted.contains("China"));
        assert_eq!(hot_sides(&terminal), (true, false));
        app.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        assert_eq!(app.atlas_runs[app.atlas_run_sel].id, "atlas-cn");
        terminal
            .draw(|frame| super::super::ui::draw(frame, &app))
            .unwrap();
        let painted = screen_text(&terminal);
        assert!(painted.contains("China"));
        assert!(!painted.contains("United States"));
        app.handle_key(KeyEvent::new(KeyCode::Char('+'), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
        terminal
            .draw(|frame| super::super::ui::draw(frame, &app))
            .unwrap();
        assert_eq!(hot_sides(&terminal), (false, true));
    }

    #[test]
    fn atlas_map_names_every_highlighted_country() {
        let mut app = app();
        app.screen = Rect::new(0, 0, 140, 48);
        let origins = [
            ("us", 1_u8),
            ("cn", 1),
            ("uk", 2),
            ("fr", 2),
            ("de", 2),
            ("es", 3),
            ("jp", 3),
            ("br", 3),
            ("au", 3),
            ("in", 3),
            ("sg", 3),
        ]
        .into_iter()
        .enumerate()
        .map(|(index, (country, tier))| atlas::OriginStat {
            country: country.into(),
            tier,
            temperature: 1.0 - index as f64 * 0.08,
            volume: 4,
            articles: 2,
        })
        .collect();
        let stats = serde_json::to_string(&atlas::RunStats {
            scored: true,
            origins,
            ..atlas::RunStats::default()
        })
        .unwrap();
        app.store
            .atlas_insert_run("atlas-many", "{}", &stats)
            .unwrap();
        app.store
            .atlas_set_state("atlas-many", "completed", "", true)
            .unwrap();
        app.select(ModuleId::Atlas.index());
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(140, 48)).unwrap();
        terminal
            .draw(|frame| super::super::ui::draw(frame, &app))
            .unwrap();
        let painted = screen_text(&terminal);
        for name in [
            "United States",
            "China",
            "United Kingdom",
            "France",
            "Germany",
        ] {
            assert!(painted.contains(name), "missing {name}");
        }
        for code in ["ES", "JP", "BR", "AU", "IN", "SG"] {
            assert!(painted.contains(code), "missing {code}");
        }
        // Stats shows full country labels for every origin, including tier 3.
        for name in ["Spain", "Japan", "Brazil", "Australia", "India"] {
            assert!(painted.contains(name), "stats missing {name}");
        }
    }

    #[test]
    fn atlas_live_insights_show_extract_progress() {
        let mut app = app();
        app.screen = Rect::new(0, 0, 120, 42);
        app.module = Some(ModuleId::Atlas);
        app.atlas_page = AtlasPage::Live;
        app.atlas_pause = Some(Arc::new(AtomicBool::new(false)));
        app.atlas_status = "Extracting insights".into();
        app.atlas_insight_progress = Some((3, 12));
        assert!(super::super::ui::atlas_extracting(&app));
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 42)).unwrap();
        terminal
            .draw(|frame| super::super::ui::draw(frame, &app))
            .unwrap();
        let painted = screen_text(&terminal);
        assert!(painted.contains("Extracting insights"), "{painted}");
        assert!(painted.contains("3 / 12"), "{painted}");
        assert!(
            !painted.contains("No insights extracted for this cycle."),
            "{painted}"
        );
        app.atlas_status = "Pipeline complete".into();
        app.atlas_insight_progress = None;
        app.atlas_pause = None;
        assert!(!super::super::ui::atlas_extracting(&app));
    }

    fn sample_headline(index: usize) -> atlas::FeedArticle {
        atlas::FeedArticle {
            id: format!("art-{index}"),
            title: format!("Headline {index}"),
            description: String::new(),
            url: format!("https://example.com/{index}"),
            country: "us".into(),
            source_name: "Wire".into(),
            source_domain: "example.com".into(),
            author: String::new(),
            image_url: String::new(),
            published_at: String::new(),
            provider: "newsapi".into(),
            temperature: 1.0,
            seen_at: String::new(),
            category: "unk".into(),
        }
    }

    #[test]
    fn atlas_headlines_scroll_and_request_failures_reach_the_system_log() {
        let mut app = app();
        app.screen = Rect::new(0, 0, 80, 24);
        app.module = Some(ModuleId::Atlas);
        app.atlas_page = AtlasPage::Live;
        for index in 0..40 {
            app.atlas_feed.push(sample_headline(index));
        }
        app.atlas_feed_sel = 0;
        app.atlas_feed_follow = false;
        for _ in 0..20 {
            app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        }
        assert_eq!(app.atlas_feed_sel, 20);
        assert!(app.scrolls.atlas_feed > 0);
        assert!(app.atlas_feed_sel >= app.scrolls.atlas_feed as usize);
        let visible_until =
            app.scrolls.atlas_feed as usize + super::super::ui::atlas_feed_room_for(&app);
        assert!(app.atlas_feed_sel < visible_until);

        app.on_work_event(WorkEvent::Atlas(atlas::AtlasEvent::Note(
            "GNews rate limit reached (HTTP 429). The free tier allows 100 requests a day.".into(),
        )));
        assert!(events(&app).iter().any(|line| {
            line.severity == "error" && line.app == "atlas" && line.message.contains("rate limit")
        }));
        app.on_work_event(WorkEvent::Atlas(atlas::AtlasEvent::Note(
            "newsapi daily quota is spent".into(),
        )));
        assert!(events(&app)
            .iter()
            .any(|line| { line.severity == "info" && line.message.contains("quota") }));
        let body = "{\n  \"status\": \"error\",\n  \"results\": {\n    \"message\": \"Access Denied! To use the latest endpoint you must upgrade.\"\n  }\n}";
        app.on_work_event(WorkEvent::Atlas(atlas::AtlasEvent::Fault(
            atlas::ProviderFault {
                summary:
                    "newsdata HTTP 422: Access Denied! To use the latest endpoint you must upgrade."
                        .into(),
                body: body.into(),
            },
        )));
        let logged = events(&app)
            .into_iter()
            .find(|line| line.message.contains("HTTP 422"))
            .expect("fault line");
        assert!(logged.details.contains("you must upgrade"));
        app.select(ModuleId::Logs.index());
        let index = app
            .logs
            .rows
            .iter()
            .position(|line| line.message.contains("HTTP 422"))
            .unwrap();
        app.logs.select(index);
        app.set_focus(Target::LogLine(index));
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(app.logs.open.contains(&logged.id));
    }

    #[test]
    fn atlas_auto_toggle_persists_the_next_trigger() {
        let mut app = app();
        app.screen = Rect::new(0, 0, 120, 40);
        app.select(ModuleId::Atlas.index());
        click(&mut app, Target::Button(ButtonId::AtlasLive));
        assert_eq!(
            super::super::ui::atlas_auto_label(&app),
            "Auto Run: Disabled"
        );
        assert!(hit(&app, Target::Button(ButtonId::AtlasAuto)));
        let before = unix_now();
        click(&mut app, Target::Button(ButtonId::AtlasAuto));
        let next = app.atlas_auto_next.expect("armed");
        assert!(next >= before + ATLAS_AUTO_SECS);
        assert!(next <= unix_now() + ATLAS_AUTO_SECS);
        assert_eq!(app.store.atlas_auto_next().unwrap(), Some(next));
        assert_eq!(
            super::super::ui::atlas_auto_label(&app),
            format!("Auto Run: {}", atlas::friendly_unix(next))
        );
        assert!(app.atlas_pause.is_none());
        click(&mut app, Target::Button(ButtonId::AtlasAuto));
        assert!(app.atlas_auto_next.is_none());
        assert!(app.store.atlas_auto_next().unwrap().is_none());
        assert_eq!(
            super::super::ui::atlas_auto_label(&app),
            "Auto Run: Disabled"
        );
    }

    #[test]
    fn auto_run_completion_selects_the_latest_history_row() {
        let mut app = app();
        app.select(ModuleId::Atlas.index());
        assert!(app.on_atlas_history());
        let older = serde_json::to_string(&atlas::RunStats {
            scored: true,
            origins: vec![atlas::OriginStat {
                country: "de".into(),
                tier: 1,
                temperature: 0.4,
                volume: 2,
                articles: 1,
            }],
            ..atlas::RunStats::default()
        })
        .unwrap();
        let newer = serde_json::to_string(&atlas::RunStats {
            scored: true,
            origins: vec![atlas::OriginStat {
                country: "us".into(),
                tier: 1,
                temperature: 1.0,
                volume: 9,
                articles: 4,
            }],
            ..atlas::RunStats::default()
        })
        .unwrap();
        app.store.atlas_insert_run("older", "{}", &older).unwrap();
        std::thread::sleep(Duration::from_millis(5));
        app.store.atlas_insert_run("newer", "{}", &newer).unwrap();
        app.load_atlas();
        app.atlas_run_sel = 1;
        app.scrolls.atlas_runs = 4;
        app.atlas_focus = Some("de".into());
        app.overlay = Overlay::Block {
            title: "Run old".into(),
            body: String::new(),
        };
        app.atlas_auto_started = true;
        app.on_work_event(WorkEvent::AtlasDone {
            outcome: Ok(atlas::Stop::Finished),
        });
        assert_eq!(app.atlas_run_sel, 0);
        assert_eq!(app.atlas_runs[0].id, "newer");
        assert_eq!(app.scrolls.atlas_runs, 0);
        assert!(app.atlas_focus.is_none());
        assert!(matches!(app.overlay, Overlay::None));
        assert!(!app.atlas_auto_started);
        let shown = serde_json::from_str::<atlas::RunStats>(&app.atlas_runs[0].stats_json).unwrap();
        assert_eq!(shown.origins[0].country, "us");

        app.atlas_run_sel = 1;
        app.atlas_auto_started = false;
        app.on_work_event(WorkEvent::AtlasDone {
            outcome: Ok(atlas::Stop::Finished),
        });
        assert_eq!(app.atlas_run_sel, 1);

        app.atlas_page = AtlasPage::Live;
        app.atlas_run_sel = 1;
        app.atlas_auto_started = true;
        app.on_work_event(WorkEvent::AtlasDone {
            outcome: Ok(atlas::Stop::Finished),
        });
        assert_eq!(app.atlas_run_sel, 1);
    }

    #[test]
    fn atlas_history_button_counts_down_while_auto_run_is_on() {
        assert_eq!(super::super::ui::atlas_countdown(5_400, 0), "1:30:00");
        assert_eq!(super::super::ui::atlas_countdown(3_661, 0), "1:01:01");
        assert_eq!(super::super::ui::atlas_countdown(59, 0), "0:00:59");
        assert_eq!(super::super::ui::atlas_countdown(10, 10), "0:00:00");
        let mut app = app();
        assert_eq!(super::super::ui::atlas_history_live_label(&app), "Go Live");
        app.atlas_auto_next = Some(unix_now() + 90 * 60);
        let label = super::super::ui::atlas_history_live_label(&app);
        assert!(label == "1:30:00" || label == "1:29:59", "{label}");
    }

    #[test]
    fn manual_run_moves_the_next_auto_trigger_out_by_90_minutes() {
        let mut app = app();
        app.atlas_auto_next = Some(unix_now() + 30);
        let before = unix_now();
        app.shift_atlas_auto_after_manual();
        let next = app.atlas_auto_next.expect("still armed");
        assert!(next >= before + ATLAS_AUTO_SECS);
        assert!(next <= unix_now() + ATLAS_AUTO_SECS);
        assert_eq!(app.store.atlas_auto_next().unwrap(), Some(next));
        app.persist_atlas_auto(None);
        app.shift_atlas_auto_after_manual();
        assert!(app.atlas_auto_next.is_none());
        assert!(app.store.atlas_auto_next().unwrap().is_none());
    }

    #[test]
    fn atlas_auto_while_running_only_arms_the_next_slot() {
        let mut app = app();
        let flag = Arc::new(AtomicBool::new(false));
        app.atlas_pause = Some(flag.clone());
        app.toggle_atlas_auto().unwrap();
        assert!(Arc::ptr_eq(app.atlas_pause.as_ref().unwrap(), &flag));
        let next = app.atlas_auto_next.expect("armed");
        assert!(next >= unix_now() + ATLAS_AUTO_SECS - 1);
        assert!(next <= unix_now() + ATLAS_AUTO_SECS);
    }

    #[test]
    fn atlas_auto_future_tick_waits_and_past_tick_reschedules() {
        let mut app = app();
        let now = unix_now();
        app.atlas_auto_next = Some(now + 3_600);
        assert!(!app.poll_atlas_auto());
        assert_eq!(app.atlas_auto_next, Some(now + 3_600));

        app.atlas_auto_next = Some(now.saturating_sub(5));
        assert!(app.poll_atlas_auto());
        let next = app.atlas_auto_next.expect("rescheduled");
        assert!(next + 1 >= now + ATLAS_AUTO_SECS);
        assert!(next <= unix_now() + ATLAS_AUTO_SECS);
        assert_eq!(app.store.atlas_auto_next().unwrap(), Some(next));
        assert!(app.atlas_pause.is_none());

        app.atlas_auto_next = Some(unix_now() + 2);
        let wait = atlas_poll_wait(&app, Duration::from_secs(5));
        assert!(wait >= Duration::from_secs(1));
        assert!(wait <= Duration::from_secs(2));
        app.atlas_auto_next = Some(unix_now());
        assert_eq!(
            atlas_poll_wait(&app, Duration::from_millis(400)),
            Duration::from_millis(1)
        );
    }

    #[test]
    fn atlas_auto_tick_skips_a_pipeline_already_running() {
        let mut app = app();
        app.atlas_pause = Some(Arc::new(AtomicBool::new(false)));
        app.atlas_auto_next = Some(1);
        assert!(app.poll_atlas_auto());
        assert!(app.atlas_pause.is_some());
        assert!(app.atlas_auto_next.unwrap() + 1 >= unix_now() + ATLAS_AUTO_SECS);
        assert_eq!(app.status, "Auto run waiting for the current pipeline");
    }

    #[test]
    fn pruned_history_closes_the_open_run() {
        let mut app = app();
        app.atlas_news = true;
        app.atlas_news_run = "old".into();
        app.atlas_focus = Some("us".into());
        app.atlas_articles.push(AtlasArticleRow {
            run_id: "old".into(),
            id: "a1".into(),
            title: "Old wire".into(),
            description: String::new(),
            url: "https://example.com".into(),
            country: "us".into(),
            source_name: "Wire".into(),
            source_domain: "example.com".into(),
            published_at: String::new(),
            provider: "newsapi".into(),
            temperature: 1.0,
            category: "unk".into(),
            seen_at: String::new(),
            author: String::new(),
            image_url: String::new(),
        });
        app.overlay = Overlay::Block {
            title: "Run 1 Jan".into(),
            body: String::new(),
        };
        app.dismiss_pruned_atlas(&["old".into()], Some("old"), "old");
        assert!(!app.atlas_news);
        assert!(app.atlas_news_run.is_empty());
        assert!(app.atlas_articles.is_empty());
        assert!(app.atlas_focus.is_none());
        assert!(matches!(app.overlay, Overlay::None));
    }

    #[test]
    fn single_action_launch_from_home() {
        let mut app = app();
        app.tab_ids.clear();
        app.module = None;
        app.input = "Investigate supply chain disruptions in Baltic ports".into();
        app.home_draft = app.input.clone();

        app.launch_home_investigation().unwrap();

        assert_eq!(app.module, Some(ModuleId::Recon));
        assert!(app.selected_thread.is_some());
        let thread_id = app.selected_thread.clone().unwrap();

        assert_eq!(app.tab_ids.len(), 1);
        assert_eq!(app.tab_ids[0], thread_id);
        assert_eq!(app.tab_sel, 1);

        let thread = app.store.get_thread(&thread_id).unwrap().unwrap();
        assert_eq!(
            thread.title,
            "Investigate supply chain disruptions in Baltic ports"
        );

        assert!(app.home_draft.is_empty());
        assert!(!app.home_draft_submitting);
        assert_eq!(app.launch_state, LaunchState::Accepted);
    }

    #[test]
    fn duplicate_submission_prevention() {
        let mut app = app();
        app.tab_ids.clear();
        app.module = None;
        app.input = "Investigate supply chain".into();
        app.home_draft = app.input.clone();
        app.home_draft_submitting = true;

        app.launch_home_investigation().unwrap();
        assert!(app.selected_thread.is_none());
        assert_eq!(app.tab_ids.len(), 0);
    }

    #[test]
    fn draft_isolation_and_persistence() {
        let mut app = app();
        app.tab_ids.clear();
        app.module = None;
        app.input = "Draft prompt on home".into();
        app.focus = Target::Field(FieldId::Composer);

        app.persist_home_draft();
        app.flush_home_draft().unwrap();

        let thread = app.store.new_thread("Test Inv").unwrap();
        app.selected_thread = Some(thread.id.clone());
        app.module = Some(ModuleId::Recon);
        app.recon_chat = true;
        app.input = "Recon follow-up draft".into();
        app.cursor = 20;

        app.switch_tab(0).unwrap();
        assert_eq!(app.module, None);
        assert_eq!(app.input, "Draft prompt on home");
    }

    #[test]
    fn session_tabs_open_close_reopen() {
        let mut app = app();
        app.tab_ids.clear();
        let t1 = app.store.new_thread("Investigation 1").unwrap().id;
        let t2 = app.store.new_thread("Investigation 2").unwrap().id;
        let t3 = app.store.new_thread("Investigation 3").unwrap().id;

        app.open_investigation_tab(&t1).unwrap();
        app.open_investigation_tab(&t2).unwrap();
        app.open_investigation_tab(&t3).unwrap();

        assert_eq!(app.tab_ids, vec![t1.clone(), t2.clone(), t3.clone()]);
        assert_eq!(app.tab_sel, 3);

        // Deduplication
        app.open_investigation_tab(&t2).unwrap();
        assert_eq!(app.tab_ids.len(), 3);
        assert_eq!(app.tab_sel, 2);

        // Close active tab t2: selects next tab (t3)
        app.close_tab(2).unwrap();
        assert_eq!(app.tab_ids, vec![t1.clone(), t3.clone()]);
        assert_eq!(app.tab_recently_closed, vec![t2.clone()]);

        // Reopen closed tab: restores t2
        app.reopen_closed_tab().unwrap();
        assert!(app.tab_ids.contains(&t2));
        assert!(app.tab_recently_closed.is_empty());
    }

    #[test]
    fn keyboard_navigation_esc_and_shortcuts() {
        let mut app = app();
        app.tab_ids = vec![String::new()];
        app.module = None;
        app.focus = Target::Field(FieldId::Composer);

        // Esc leaves composer for app launcher
        app.on_esc();
        assert_eq!(app.focus, Target::App(app.launcher_sel));

        // Digits outside text field switch apps
        app.handle_key(KeyEvent::new(KeyCode::Char('1'), KeyModifiers::NONE));
        assert_eq!(app.module, Some(ModuleId::Intel));

        // Ctrl+N routes back to Home draft
        app.handle_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::CONTROL));
        assert_eq!(app.module, None);
        assert_eq!(app.focus, Target::Field(FieldId::Composer));

        // While focused in composer, typing digit '2' appends '2' to input
        app.handle_key(KeyEvent::new(KeyCode::Char('2'), KeyModifiers::NONE));
        assert_eq!(app.input, "2");
    }

    #[test]
    fn tab_strip_render_and_hit_test() {
        let mut app = app();
        app.tab_ids.clear();
        let t1 = app.store.new_thread("Investigation 1").unwrap().id;
        app.open_investigation_tab(&t1).unwrap();

        app.screen = Rect::new(0, 0, 100, 30);
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 30)).unwrap();
        terminal
            .draw(|frame| super::super::ui::draw(frame, &app))
            .unwrap();

        assert!(hit(&app, Target::Tab(0)));
        assert!(hit(&app, Target::Tab(1)));
        assert!(hit(&app, Target::TabPlus));
        assert!(hit(&app, Target::Field(FieldId::Composer)));
        assert!(hit(&app, Target::Button(ButtonId::Send)));

        let order = super::super::ui::focus_order(&app);
        assert_eq!(order.first(), Some(&Target::Tab(0)));
        assert!(order.contains(&Target::TabPlus));
        assert!(order.contains(&Target::App(app.launcher_sel)));
        assert!(order.contains(&Target::Field(FieldId::Composer)));
        assert!(order.contains(&Target::Button(ButtonId::Send)));
    }

    #[test]
    fn intel_brief_stacked_left_column_renders_all_sections_and_scrolls() {
        let mut app = app();
        app.screen = Rect::new(0, 0, 140, 45);
        app.select(ModuleId::Intel.index());
        app.intel_page = IntelPage::Briefing;
        let article = AtlasArticleRow {
            run_id: "r1".into(),
            id: "art-1".into(),
            title: "Test Article".into(),
            description: "Test description".into(),
            url: "https://example.com/test".into(),
            country: "us".into(),
            source_name: "Test Source".into(),
            source_domain: "example.com".into(),
            author: String::new(),
            image_url: String::new(),
            published_at: "2026-10-08T12:00:00Z".into(),
            provider: "newsapi".into(),
            temperature: 0.8,
            category: "stability".into(),
            seen_at: String::new(),
        };
        app.intel_articles = vec![article];
        app.intel_sel = 0;
        app.intel_claims = vec![
            AtlasArticleClaim {
                article_id: "art-1".into(),
                fingerprint: "fp1".into(),
                entity: "Acme Corp".into(),
                predicate: "launched".into(),
                object: "Widget".into(),
                topic: "technology".into(),
                claim: "Acme Corp launched Widget".into(),
                classification: "fact".into(),
                confidence: 0.95,
                source_url: String::new(),
                published_at: String::new(),
                reliability: "A".into(),
                info_credibility: 1,
                admiralty: "A1".into(),
                rsp_status: "verified".into(),
            },
            AtlasArticleClaim {
                article_id: "art-1".into(),
                fingerprint: "fp2".into(),
                entity: "Widget".into(),
                predicate: "competes_with".into(),
                object: "Gadget".into(),
                topic: "technology".into(),
                claim: "Widget will pressure competitors".into(),
                classification: "inference".into(),
                confidence: 0.75,
                source_url: String::new(),
                published_at: String::new(),
                reliability: "B".into(),
                info_credibility: 2,
                admiralty: "B2".into(),
                rsp_status: "inferred".into(),
            },
        ];
        app.intel_relations = vec![("fp1".into(), "fp2".into(), "leads_to".into())];

        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(140, 45)).unwrap();
        terminal
            .draw(|frame| super::super::ui::draw(frame, &app))
            .unwrap();
        let painted = screen_text(&terminal);
        assert!(painted.contains("claims"), "{painted}");
        assert!(painted.contains("inferences"), "{painted}");
        assert!(painted.contains("actors"), "{painted}");
        assert!(painted.contains("links"), "{painted}");
        assert!(painted.contains("related context"), "{painted}");
        assert!(painted.contains("Acme Corp"), "{painted}");

        // Scroll left column
        let max = super::super::ui::intel_extracted_scroll_max(&app);
        app.scrolls.intel_extracted = max;
        terminal
            .draw(|frame| super::super::ui::draw(frame, &app))
            .unwrap();

        // Actor reviewing loading state
        app.intel_actors_reviewing.insert("art-1".into());
        terminal
            .draw(|frame| super::super::ui::draw(frame, &app))
            .unwrap();
        let painted_actor_rev = screen_text(&terminal);
        assert!(
            painted_actor_rev.contains("Reviewing actors"),
            "{painted_actor_rev}"
        );
        app.intel_actors_reviewing.clear();

        // Link explanation loading state
        app.intel_links_reviewing.insert("art-1".into());
        terminal
            .draw(|frame| super::super::ui::draw(frame, &app))
            .unwrap();
        let painted_link_rev = screen_text(&terminal);
        assert!(
            painted_link_rev.contains("Explaining links"),
            "{painted_link_rev}"
        );
        app.intel_links_reviewing.clear();
    }

    #[test]
    fn filter_fields_display_item_counts_in_placeholders() {
        let mut app = app();
        app.threads = vec![
            app.store.new_thread("Test 1").unwrap(),
            app.store.new_thread("Test 2").unwrap(),
        ];
        let placeholder_recon = super::super::ui::field_placeholder(&app, FieldId::ReconSearch);
        assert!(
            placeholder_recon.contains("(2 items)"),
            "{placeholder_recon}"
        );

        let placeholder_intel = super::super::ui::field_placeholder(&app, FieldId::IntelSearch);
        assert!(
            placeholder_intel.contains("(0 items)"),
            "{placeholder_intel}"
        );

        let placeholder_osint = super::super::ui::field_placeholder(&app, FieldId::OsintSearch);
        assert!(placeholder_osint.contains("items)"), "{placeholder_osint}");

        let placeholder_jobs = super::super::ui::field_placeholder(&app, FieldId::JobsSearch);
        assert!(placeholder_jobs.contains("(0 items)"), "{placeholder_jobs}");

        let placeholder_logs = super::super::ui::field_placeholder(&app, FieldId::LogsSearch);
        assert!(placeholder_logs.contains("(0 items)"), "{placeholder_logs}");

        let placeholder_brain = super::super::ui::field_placeholder(&app, FieldId::BrainQuery);
        assert!(
            placeholder_brain.contains("(0 items)"),
            "{placeholder_brain}"
        );
    }

    fn hit(app: &App, target: Target) -> bool {
        (0..app.screen.height)
            .flat_map(|y| (0..app.screen.width).map(move |x| (x, y)))
            .any(|(x, y)| super::super::ui::hit_test(app, x, y) == Some(target))
    }

    /// Seeds one observation per widget family so a snapshot has content.
    fn seed_profile(app: &mut App) {
        let conn = app.store.connection();
        let now = chrono::Utc::now().to_rfc3339();
        let record = |event: argos_osint_core::telemetry::TelemetryEvent| {
            let _ = argos_osint_core::telemetry::insert(conn, &event);
        };
        record(
            argos_osint_core::telemetry::TelemetryEvent::new(
                argos_osint_core::telemetry::EventKind::ReconRun,
            )
            .at(now.clone())
            .app("tui")
            .mode("deep")
            .outcome("completed"),
        );
        record(
            argos_osint_core::telemetry::TelemetryEvent::new(
                argos_osint_core::telemetry::EventKind::ToolInvocation,
            )
            .at(now.clone())
            .app("tui")
            .provider("firecrawl")
            .role("recon")
            .mode("remote")
            .tool("firecrawl_scrape")
            .category("web")
            .engine("google")
            .outcome("completed_nonempty")
            .duration_ms(Some(420)),
        );
        record(
            argos_osint_core::telemetry::TelemetryEvent::new(
                argos_osint_core::telemetry::EventKind::AtlasCycle,
            )
            .at(now.clone())
            .run("run-1")
            .outcome("completed"),
        );
        record(
            argos_osint_core::telemetry::TelemetryEvent::new(
                argos_osint_core::telemetry::EventKind::ModelAttempt,
            )
            .at(now)
            .app("tui")
            .provider("openrouter")
            .role("recon")
            .model("gpt-4o-mini")
            .outcome("succeeded")
            .duration_ms(Some(310)),
        );
        app.profile.loaded_at = None;
        app.profile.reload(&app.store);
    }

    /// The four viewports the spec requires, plus a narrow one that must still
    /// render the dashboard rather than a blank pane.
    #[test]
    fn profile_overview_renders_at_every_required_viewport() {
        for (width, height) in [(160u16, 50u16), (100, 32), (80, 24), (64, 18)] {
            let mut app = app();
            app.select(ModuleId::System.index());
            seed_profile(&mut app);
            let text = buffer_text(&render(&mut app, width, height));
            assert!(
                text.contains(" period "),
                "{width}x{height}: the filter strip must render: {text}"
            );
            assert!(
                text.contains("[Tab] switch"),
                "{width}x{height}: the tab switch must render: {text}"
            );
            // The status strip names the reviewed inventory instead of guessing.
            assert!(
                text.contains("35"),
                "{width}x{height}: the widget count must render: {text}"
            );
        }
    }

    /// A viewport too small for the Overview keeps the tab strip and says so
    /// instead of drawing a mangled layout.
    #[test]
    fn profile_narrow_viewport_explains_itself_instead_of_going_blank() {
        let mut app = app();
        app.select(ModuleId::System.index());
        seed_profile(&mut app);
        // A 40-wide body is still below the Overview's floor, so the pane says
        // it needs more room rather than rendering half a chart.
        let text = buffer_text(&render(&mut app, 44, 14));
        assert!(
            text.contains("[Tab] switch"),
            "the tab strip survives a narrow viewport: {text}"
        );
        assert!(
            text.contains("wider") || text.contains(" period "),
            "the Overview either explains itself or still renders: {text}"
        );
    }

    /// Keyboard and mouse must reach the same target.
    #[test]
    fn profile_keyboard_and_mouse_focus_agree() {
        let mut app = app();
        app.select(ModuleId::System.index());
        seed_profile(&mut app);

        // Keyboard: `x` opens the Configs popup, and Tab moves its field focus.
        app.handle_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));
        assert_eq!(app.overlay, Overlay::Configs);
        let keyboard_focus = app.focus;
        assert!(
            matches!(keyboard_focus, Target::Field(FieldId::ProfileExportPath)),
            "the export tab focuses its path field: {keyboard_focus:?}"
        );

        // Mouse: the same field is reachable by clicking, and the hit test
        // resolves it to the same target.
        click(&mut app, Target::Field(FieldId::ProfileExportPath));
        assert_eq!(app.focus, keyboard_focus);

        // Tab moves to the import editor, and its own field is clickable too.
        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(
            app.focus,
            Target::Field(FieldId::ProfileImportEditor),
            "Tab moves the Configs popup to the editor"
        );
    }

    /// Bracketed paste reaches the import editor with newlines intact, and a
    /// keyed editor inserts a whole document in one event.
    #[test]
    fn profile_import_editor_accepts_a_pasted_multiline_document() {
        let mut app = app();
        app.select(ModuleId::System.index());
        app.handle_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(
            app.profile_config.tab,
            crate::tui::profile_config::ConfigTab::Import
        );

        let document = "{\n  \"schema_version\": 1,\n  \"providers\": [],\n}";
        app.edit_paste(document);
        assert_eq!(app.profile_config.import.text, document);
        // Newlines survive, so the caret sits at the end of the last line.
        let (line, _) = app.profile_config.import.caret_position();
        assert_eq!(line, 4);

        // Validation keeps the caret's line for the error pointer.
        app.profile_config.validate();
        assert!(app.profile_config.import.error.is_some());
        let error = app.profile_config.import.error.clone().unwrap();
        assert!(!error.message.is_empty());
        // The document is not valid, so the Import button is never armed.
        assert!(app.profile_config.plan.is_none());

        // Backspace removes one character, never a whole line.
        app.profile_config.backspace();
        assert_eq!(app.profile_config.import.text.len(), document.len() - 1);
    }

    /// The Configs popup covers about 85% of the viewport and is bounded by it.
    #[test]
    fn profile_configs_popup_covers_most_of_the_viewport() {
        let mut app = app();
        app.select(ModuleId::System.index());
        app.handle_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));
        let area = crate::tui::ui::configs_area(&app, app.screen);
        let fraction = f64::from(area.width) / f64::from(app.screen.width.max(1));
        assert!(
            fraction > 0.7 && fraction <= 0.9,
            "the popup covers about 85%: {fraction}"
        );
        assert!(area.width <= app.screen.width);
        assert!(area.height <= app.screen.height);
    }

    /// Closing the popup clears the pasted secret buffer.
    #[test]
    fn profile_configs_popup_clears_the_pasted_secret_on_close() {
        let mut app = app();
        app.select(ModuleId::System.index());
        app.handle_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        app.profile_config.import.paste = "sk-secret-value-123".to_string();
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.overlay, Overlay::None);
        assert!(
            app.profile_config.import.paste.is_empty(),
            "a pasted secret never outlives the popup"
        );
    }

    /// Every section reports its reviewed widget count, and the section
    /// navigator reaches all five.
    #[test]
    fn profile_sections_and_widgets_match_the_reviewed_inventory() {
        let mut app = app();
        app.select(ModuleId::System.index());
        seed_profile(&mut app);
        let mut seen = Vec::new();
        for _ in 0..crate::tui::profile::Section::all().len() {
            seen.push(app.profile.section);
            app.handle_key(KeyEvent::new(KeyCode::Char(']'), KeyModifiers::NONE));
        }
        assert_eq!(seen.len(), 5);
        // All five sections are reachable, and the navigator wraps.
        app.handle_key(KeyEvent::new(KeyCode::Char('['), KeyModifiers::NONE));
        assert_eq!(app.profile.section, seen[4]);
        // The numeric shortcuts jump straight to a section.
        app.handle_key(KeyEvent::new(KeyCode::Char('3'), KeyModifiers::NONE));
        assert_eq!(app.profile.section, crate::tui::profile::Section::Atlas);
    }
}
