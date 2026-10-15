//! App state and keyboard routing for the Argos terminal shell.

use super::brain_detail::RelatedTab;
use super::brain_detail::{AssociationItem, AssociationTarget};
use super::tracked;
use super::ui::nudge;
use anyhow::Result;
use argos_osint_core::brain::{Memory, MemorySource, ScoredMemory};
use argos_osint_core::hardware::{self, HardwareProfile};
use argos_osint_core::intel_recon::{self, ArticleBodyRow, ReportMode};
use argos_osint_core::job_registry::{CancelRequest, JobSpec};
use argos_osint_core::paths;
use argos_osint_core::provider::{self, ListedModel, SettingsFile};
use argos_osint_core::related_global::RelatedScope;
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
            Self::Intel => "Browse classified headlines and run article briefings",
            Self::Recon => "Run evidence-driven OSINT investigations on any target",
            Self::Brain => "Explore saved memories, their paths, and related evidence",
            Self::Atlas => "Collect and classify global headlines on a live map",
            Self::Jobs => "Track background jobs, timing, retries, and cancels",
            Self::Logs => "Follow durable events, errors, and per-job logs",
            Self::Osint => "Browse, configure, and run OSINT tools",
            Self::Providers => "Set provider accounts and per-role model fallbacks",
            Self::System => "Usage analytics, host details, and config transfer",
        }
    }
}

/// Rotating Home composer examples, one per Recon target type the tool
/// catalog covers (`osint::contracts::IntelligenceCategory`). `(kind, example)`.
pub const RECON_TIPS: &[(&str, &str)] = &[
    ("domain", "What is known about example.org?"),
    ("person", "Who is Jane Example, CEO of Acme Robotics?"),
    ("username", "Which sites use the handle @jdoe42?"),
    ("email", "Where is jane@example.com registered?"),
    ("phone", "What is publicly linked to +1 202 555 0143?"),
    ("ip", "Who operates 203.0.113.7 and what does it expose?"),
    ("company", "Who owns Acme Robotics and what has it filed?"),
    ("news", "What is the latest on the Acme Robotics recall?"),
    ("legal", "Which court cases or dockets name Acme Robotics?"),
    (
        "social",
        "What has @acmerobotics posted on Instagram lately?",
    ),
    ("url", "Is https://example.com/login a phishing page?"),
    (
        "history",
        "How has example.org changed in the Wayback Machine?",
    ),
    ("code", "Which GitHub repos belong to acme-robotics?"),
    (
        "cve",
        "Is CVE-2024-3094 exploited, and what does it affect?",
    ),
    (
        "package",
        "Does the npm package left-pad have known vulnerabilities?",
    ),
    (
        "bitcoin",
        "What has Bitcoin address 1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa received?",
    ),
    (
        "place",
        "What is near 1600 Pennsylvania Ave NW, Washington?",
    ),
    (
        "publisher",
        "How reliable is the Daily Example as a source?",
    ),
];
/// How long one Home composer tip stays before the next one.
pub const RECON_TIP_EVERY: Duration = Duration::from_secs(6);

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
    pub chat: usize,
    pub threads: u16,
    pub home: u16,
    pub roles: u16,
    pub fallbacks: u16,
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
    /// Vertical scroll offset for the Recon investigation context panel.
    pub recon_context: u16,
}

#[derive(Clone, Debug)]
struct TopologyOrigin {
    module: Option<ModuleId>,
    brain_mode: BrainListMode,
    intel_page: IntelPage,
    article_id: Option<String>,
    article_revision: Option<(String, String)>,
    compact_pages: [usize; 9],
    scrolls: Scrolls,
    focus: Target,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ChoiceKind {
    Provider,
    Model,
    IntelDay,
    Investigation,
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
    Topology,
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
    /// Apply or reset the Brief fast preset (Claims/Links → Summarization).
    BriefFastPreset,
    /// Select the next Brief operation to edit.
    BriefOpCycle,
    /// Save the Provider/Model fields as the selected operation's override.
    BriefOpSave,
    /// Remove the selected operation's override (it inherits its role again).
    BriefOpReset,
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
    /// Resume retained collection/classification and native metadata work.
    AtlasResume,
    AtlasNewsFeed,
    AtlasWorld,
    AtlasNews,
    IntelDay,
    IntelExtractionRetry,
    IntelExtractionJob,
    IntelBodyRetry,
    IntelBodyRefresh,
    IntelVisit,
    JumpLatest,
    IntelJobOpen,
    CreateMemory,
    /// Cycle the Brain browse filter (All → Claims → Actors).
    BrainKind,
    /// Toggle Related articles between all retained cycles and this cycle.
    RelatedScope,
    BrainBack,
    /// Back in the memory detail history (or to the Brain list).
    BrainDetailBack,
    BrainRelatedFirst,
    BrainRelatedSecond,
    BrainRelatedThird,
    BrainAssociationsMore,
    IntelRelatedMore,
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
    JobsClearCompleted,
    JobsClearStopped,
    JobsClearFailed,
    LogsLevel,
    LogsApp,
    LogsFollow,
    LogsOpenJob,
    LogsBack,
    RouterSave,
    RouterVerify,
    RouterAdvanced,
    IntelBodyMore,
    GoogleSave,
    GoogleVerify,
    GoogleAdvanced,
    NvidiaSave,
    NvidiaVerify,
    NvidiaAdvanced,
    RefreshHardware,
    ResumeSessionConfirm,
    ResumeSessionDismiss,
    BrainMapTopology,
    TopologyBack,
    TopologyOpenMemory,
    TopologyViewEvidence,
    TopologyScopeAll,
    TopologyScopeThis,
    TopologyDepth1,
    TopologyDepth2,
    TopologyDepth3,
    TopologyCenterSelected,
    TopologyFitGraph,
    TopologyAnalyzeScope,
    TopologyRecenter,
    TopologyZoomIn,
    TopologyZoomOut,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Target {
    PaneTab(usize),
    ProfileTab(usize),
    ProfileApp(usize),
    ProfileAction(usize),
    ProfileCard(usize),
    ProfileChoice(usize),
    ProfileBucket(usize),
    ProfilePanelBucket(usize, usize),
    ProfilePanelRow(usize, usize),
    ProfileReport,
    ProfileTable(usize),
    ProfileTableRow(usize, usize),
    ProfileRow(usize),
    ProfileSystem(usize),
    ConfigAction(usize),
    App(usize),
    Home,
    ProviderTab(ProviderPage),
    Memory(usize),
    Thread(usize),
    Tool(usize),
    ToolCategory(&'static str),
    OsintResponse,
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
    /// The path graph section of the memory detail (scroll focus).
    DetailPath,
    /// One Related memory row in the memory detail.
    RelatedRow(usize),
    IntelRelatedArticle(usize),
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
    IntelFullArticle,
    IntelPreview,
    TopologyGraph,
    TopologyNode(usize),
    TopologyEdge(usize),
    TopologyRelated(usize),
    TopologyPanel(usize),
    TopologyInspectorTab(usize),
    TopologyFilter(usize),
    TopologyEndpoint(usize),
    TopologyInspector,
    TopologyRelationships,
    IntelTopologyClaim(usize),
    IntelTopologyActor(usize),
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
struct LoadedBrainDetail {
    memory: Memory,
    graph: recon::MemoryGraph,
    typed: Vec<String>,
    insight: Option<recon::InsightView>,
    claim: bool,
    associations: argos_osint_core::brain_browse::Associations,
}

#[derive(Debug)]
enum DashboardSnapshot {
    Logs(Box<super::logs::LogsView>),
    Jobs(Box<super::jobs::JobsView>),
    Counts(argos_osint_core::jobs_view::EventCounts),
}

#[derive(Debug)]
enum WorkEvent {
    LogsCleared {
        outcome: Result<usize, String>,
    },
    JobsCleared {
        outcome: Result<String, String>,
    },
    BrainPreviewLoaded {
        generation: u64,
        memory_id: String,
        outcome: Result<Option<recon::InsightView>, String>,
    },
    ManualHistoryLoaded {
        outcome: Result<Vec<recon::Call>, String>,
    },
    MemoryDeleted {
        memory_id: String,
        outcome: Result<bool, String>,
    },
    ReconLoaded {
        thread_id: String,
        outcome: Result<Box<super::db_reads::ReconSnapshot>, String>,
    },
    ThreadsLoaded {
        query: String,
        outcome: Result<(Vec<recon::Thread>, HashMap<String, String>), String>,
    },
    IntelListLoaded {
        key: (String, String, String),
        outcome: Result<(Vec<String>, String, Vec<AtlasArticleRow>), String>,
    },
    AtlasArticlesLoaded {
        run_id: String,
        outcome: Result<Vec<AtlasArticleRow>, String>,
    },
    ThreadDeleted {
        thread_id: String,
        outcome: Result<usize, String>,
    },
    DashboardLoaded {
        module: Option<ModuleId>,
        key: String,
        outcome: Result<Box<DashboardSnapshot>, String>,
    },
    IntelBriefLoaded {
        key: (String, String, RelatedScope, Option<String>, usize),
        outcome: Result<Box<super::db_reads::IntelBriefSnapshot>, String>,
    },
    AtlasLoaded {
        generation: u64,
        outcome: Result<(Vec<AtlasRunRow>, Vec<String>), String>,
    },
    AtlasDeleted {
        run_id: String,
        outcome: Result<bool, String>,
    },
    BrainDetailLoaded {
        generation: u64,
        outcome: Result<Box<LoadedBrainDetail>, String>,
    },
    TopologyLoaded {
        generation: u64,
        seed: argos_osint_core::brain_topology::TopologySeed,
        outcome: Result<Box<argos_osint_core::brain_topology::TopologySnapshot>, String>,
    },
    BrainLoaded {
        generation: u64,
        outcome: Result<Vec<Memory>, String>,
    },
    BrainMaintenance {
        outcome: Result<(), String>,
    },
    ProfileLoaded {
        generation: u64,
        filters: argos_osint_core::profile_stats::StatFilters,
        outcome: Result<Box<argos_osint_core::profile_stats::ProfileSnapshot>, String>,
        options: Vec<(String, Vec<String>)>,
    },
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
        outcome: std::result::Result<Vec<AssociationItem>, String>,
    },
    Atlas(atlas::AtlasEvent),
    AtlasDone {
        outcome: std::result::Result<atlas::Stop, String>,
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
    pub view_stamp: String,
    pub entries: Vec<FocusEntry>,
    pub scopes: Vec<ratatui::layout::Rect>,
    pub current_scope: usize,
}

impl LayoutRegistry {
    pub fn clear(&mut self) {
        self.view_stamp.clear();
        self.entries.clear();
        self.scopes.clear();
        self.current_scope = 0;
    }

    pub fn push_scope(&mut self, rect: ratatui::layout::Rect) -> usize {
        let id = self.scopes.len() + 1;
        self.scopes.push(rect);
        self.current_scope = id;
        id
    }

    pub fn register(&mut self, target: Target, rect: ratatui::layout::Rect) {
        if rect.width == 0 || rect.height == 0 {
            return;
        }
        self.entries.push(FocusEntry {
            target,
            rect,
            scope: self.current_scope,
            scrollable: false,
        });
    }

    pub fn target_at(&self, x: u16, y: u16) -> Option<Target> {
        self.entries
            .iter()
            .rev()
            .find(|e| {
                x >= e.rect.x
                    && x < e.rect.x + e.rect.width
                    && y >= e.rect.y
                    && y < e.rect.y + e.rect.height
            })
            .map(|e| e.target)
    }
}

#[derive(Clone)]
enum RelatedArticleBack {
    Article {
        article: Box<AtlasArticleRow>,
        related: Box<super::related_articles::ArticleRelatedView>,
        extracted_scroll: u16,
        brief_scroll: u16,
        full_scroll: u16,
        focus: Target,
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
    pub intel_search: String,
    pub osint_search: String,
    pub tool_expanded: HashSet<&'static str>,
    pub response_scroll: usize,
    pub intel_preview_scroll: usize,
    pub tool_dataset_status: String,
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
    pub investigation_events: Vec<argos_osint_core::investigation::InvestigationEvent>,
    pub compact_pages: [usize; 9],
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
    answer_memory_summaries: HashMap<String, String>,
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
    /// Selected Brief operation in the Defaults per-operation editor.
    pub brief_op_sel: usize,
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
    pub palette_recent: Vec<String>,
    pub palette_restore: Option<Target>,
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
    /// Index into [`RECON_TIPS`] shown as the empty Home composer placeholder.
    pub recon_tip: usize,
    /// When the current tip appeared; rotation waits [`RECON_TIP_EVERY`].
    pub recon_tip_at: Instant,
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
    pub article_images: crate::tui::article_image::ArticleImages,
    pub intel_sel: usize,
    pub intel_claims: Vec<AtlasArticleClaim>,
    pub intel_relations: Vec<(String, String, String)>,
    pub intel_body: Option<ArticleBodyRow>,
    pub intel_body_message: String,
    pub intel_body_read_error: Option<String>,
    pub intel_body_owner: Option<(String, Option<String>)>,
    pub intel_body_require_commit: bool,
    pub intel_full_collapsed: bool,
    pub(crate) intel_body_running: HashSet<String>,
    /// Active Intel brief preparation chain for the open article.
    pub(crate) intel_brief_job: Option<String>,
    /// Shared stage views (Full article → Claims → Inferences → Links).
    pub(crate) intel_brief_stages: Vec<intel_recon::brief_pipeline::StageView>,
    /// Committed stage outputs of the current chain (by operation).
    pub(crate) intel_brief_outputs: HashMap<String, serde_json::Value>,
    /// Candidate article titles for published leads (by article id).
    pub(crate) intel_brief_lead_titles: HashMap<String, String>,
    /// Publication footer ("Published · …" / "Pending · after Links").
    pub(crate) intel_brief_publication: String,
    /// Article ids whose cleaned-body insight re-extract is still in flight.
    pub(crate) intel_insights_running: HashSet<String>,
    pub(crate) intel_links_reviewing: HashSet<String>,
    pub(crate) intel_insight_lifecycle: String,
    pub(crate) intel_extraction_progress:
        Option<argos_osint_core::investigation::insight_execution::Progress>,
    pub(crate) intel_actors: Vec<argos_osint_core::atlas_metadata::ArticleActor>,
    pub(crate) intel_actor_state: String,
    pub(crate) intel_refresh_status: String,
    pub(crate) intel_evidence_confidence: Option<intel_recon::EvidenceConfidence>,
    pub(crate) intel_metadata_present: bool,
    pub(crate) intel_publisher_prior: Option<argos_osint_core::atlas_metadata::PublisherPrior>,
    pub(crate) intel_related: super::related_articles::ArticleRelatedView,
    related_back: Vec<RelatedArticleBack>,
    brain_destination_back: Option<super::brain_detail::DetailSnapshot>,
    pub(crate) intel_link_explanations: HashMap<(String, String), String>,
    pub gnews_key: String,
    pub gnews_fallback: String,
    pub newsdata_key: String,
    pub newsdata_fallback: String,
    pub currents_key: String,
    pub currents_fallback: String,
    pub whoxy_key: String,
    pub whoxy_fallback: String,
    pub brain_graph: recon::MemoryGraph,
    /// Typed payload lines of the open memory (kind header first).
    pub brain_typed: Vec<String>,
    /// Brain list kind filter; `None` = default evidence listing.
    pub brain_kind: Option<argos_osint_core::memory_metadata::MemoryKind>,
    /// Related articles scope shared by Intel and Brain (all cycles default).
    pub related_scope: RelatedScope,
    brain_graph_for: Option<String>,
    /// Open memory detail (graph, Related, Summary), independent of `memory_sel`.
    pub brain_detail: super::brain_detail::BrainDetail,
    /// Brain Memory Topology interactive state.
    pub topology_state: super::topology::TopologyState,
    topology_origin: Option<TopologyOrigin>,
    topology_generation: u64,
    topology_reading: bool,
    pub(crate) topology_loading: bool,
    pub(crate) topology_error: Option<String>,
    pub(crate) topology_origin_label: String,
    /// Total saved memories, to explain an active Find filter.
    pub memory_total: usize,
    /// Last memory-list read failure; the previous list stays on screen.
    pub memory_error: Option<String>,
    /// False until the first memory list read finished.
    pub memories_loaded: bool,
    background_db: Option<PathBuf>,
    atlas_deleting: Option<String>,
    atlas_reading: bool,
    atlas_generation: u64,
    intel_reading: bool,
    intel_related_limit: usize,
    dashboard_reading: bool,
    pending_job: Option<String>,
    jobs_clearing: bool,
    logs_clearing: bool,
    threads_deleting: HashSet<String>,
    memories_deleting: HashSet<String>,
    recon_reading: bool,
    threads_reading: bool,
    intel_list_reading: bool,
    atlas_articles_reading: bool,
    manual_reading: bool,
    manual_pending: bool,
    threads_pending: bool,
    recon_pending: bool,
    intel_list_pending: bool,
    brain_preview_reading: bool,
    brain_preview_target: Option<String>,
    brain_preview_for: Option<String>,
    brain_cache: Vec<Memory>,
    brain_generation: u64,
    brain_reading: Option<u64>,
    brain_dirty: bool,
    brain_prefer: Option<String>,
    brain_detail_generation: u64,
    brain_detail_reading: bool,
    brain_detail_target: Option<String>,
    brain_detail_restore: Option<super::brain_detail::DetailSnapshot>,
    brain_detail_back: bool,
    brain_associations: Option<argos_osint_core::brain_browse::Associations>,
    brain_detail_claim: bool,
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

fn valid_browser_url(value: &str) -> bool {
    url::Url::parse(value)
        .is_ok_and(|url| matches!(url.scheme(), "http" | "https") && url.host_str().is_some())
}

fn open_external_url(url: &str) -> Result<()> {
    let parsed = url::Url::parse(url)?;
    anyhow::ensure!(
        matches!(parsed.scheme(), "http" | "https") && parsed.host_str().is_some(),
        "Article URL must use HTTP or HTTPS"
    );
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
        std::process::Command::new("rundll32")
            .args(["url.dll,FileProtocolHandler", url])
            .spawn()?;
        return Ok(());
    }
    #[allow(unreachable_code)]
    Err(anyhow::anyhow!("no URL opener for this platform"))
}

impl App {
    /// The Home composer placeholder, or `None` once the user has typed.
    pub fn recon_tip_text(&self) -> Option<&'static str> {
        self.input
            .is_empty()
            .then(|| RECON_TIPS[self.recon_tip % RECON_TIPS.len()].1)
    }

    /// Advance to the next tip once [`RECON_TIP_EVERY`] has passed with an
    /// empty composer. Typing freezes rotation; the clock restarts so a
    /// cleared composer shows each tip for a full interval. Returns true
    /// when the visible tip changed.
    pub fn advance_recon_tip(&mut self, now: Instant) -> bool {
        if !self.input.is_empty() {
            self.recon_tip_at = now;
            return false;
        }
        if now.saturating_duration_since(self.recon_tip_at) < RECON_TIP_EVERY {
            return false;
        }
        self.recon_tip = (self.recon_tip + 1) % RECON_TIPS.len();
        self.recon_tip_at = now;
        true
    }

    pub fn boot() -> Result<Self> {
        paths::ensure_home()?;
        // Finish any configuration commit a previous process left half applied.
        let _ = argos_osint_core::config_commit::recover_at_startup();
        let store = Store::open_ui(&paths::db_path())?;
        store.recover_runs()?;
        let threads = Vec::new();
        let thread_states = HashMap::new();
        let selected_thread = store.last_thread()?;
        let messages = Vec::new();
        let draft = selected_thread
            .as_ref()
            .and_then(|id| store.get_thread(id).ok().flatten())
            .map(|t| t.draft)
            .unwrap_or_default();
        let chat_scroll = selected_thread
            .as_ref()
            .and_then(|id| store.get_thread(id).ok().flatten())
            .map(|t| t.scroll.max(0) as usize)
            .unwrap_or_default();
        let calls = Vec::new();
        let manual_runs: Vec<recon::Call> = Vec::new();
        let manual_run_pos = manual_runs.len().saturating_sub(1);
        let osint_result = manual_runs
            .get(manual_run_pos)
            .and_then(|call| call.result.clone().map(|result| (call.id.clone(), result)));
        let tool_enabled = osint::registry()
            .iter()
            .map(|tool| store.tool_enabled(tool.id))
            .collect::<Result<Vec<_>>>()?;
        let memories = Vec::new();
        let memory_total = 0;
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
            tool_expanded: crate::tui::tool_catalog::initial_expansion(),
            response_scroll: 0,
            intel_preview_scroll: 0,
            tool_dataset_status: String::new(),
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
            investigation_events: Vec::new(),
            compact_pages: [0; 9],
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
            answer_memory_summaries: HashMap::new(),
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
            brief_op_sel: 0,
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
            article_images: crate::tui::article_image::ArticleImages::default(),
            palette_recent: Vec::new(),
            palette_restore: None,
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
            recon_tip: 0,
            recon_tip_at: Instant::now(),
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
            intel_body_read_error: None,
            intel_body_owner: None,
            intel_body_require_commit: false,
            intel_full_collapsed: false,
            intel_body_running: HashSet::new(),
            intel_brief_job: None,
            intel_brief_stages: Vec::new(),
            intel_brief_outputs: HashMap::new(),
            intel_brief_lead_titles: HashMap::new(),
            intel_brief_publication: String::new(),
            intel_insights_running: HashSet::new(),
            intel_links_reviewing: HashSet::new(),
            intel_insight_lifecycle: "legacy/unassessed".into(),
            intel_extraction_progress: None,
            intel_actors: Vec::new(),
            intel_actor_state: "pending".into(),
            intel_refresh_status: "Waiting".into(),
            intel_evidence_confidence: None,
            intel_metadata_present: false,
            intel_publisher_prior: None,
            intel_related: Default::default(),
            related_back: Vec::new(),
            brain_destination_back: None,
            intel_link_explanations: HashMap::new(),
            brain_graph: recon::MemoryGraph::default(),
            brain_typed: Vec::new(),
            brain_kind: None,
            related_scope: RelatedScope::default(),
            brain_graph_for: None,
            brain_detail: Default::default(),
            topology_state: Default::default(),
            topology_origin: None,
            topology_generation: 0,
            topology_reading: false,
            topology_loading: false,
            topology_error: None,
            topology_origin_label: String::new(),
            memory_total,
            memory_error: None,
            memories_loaded: false,
            background_db: Some(paths::db_path()),
            atlas_deleting: None,
            atlas_reading: false,
            atlas_generation: 0,
            intel_reading: false,
            intel_related_limit: 20,
            dashboard_reading: false,
            pending_job: None,
            jobs_clearing: false,
            logs_clearing: false,
            threads_deleting: HashSet::new(),
            memories_deleting: HashSet::new(),
            recon_reading: false,
            threads_reading: false,
            intel_list_reading: false,
            atlas_articles_reading: false,
            manual_reading: false,
            manual_pending: false,
            threads_pending: false,
            recon_pending: false,
            intel_list_pending: false,
            brain_preview_reading: false,
            brain_preview_target: None,
            brain_preview_for: None,
            brain_cache: Vec::new(),
            brain_generation: 0,
            brain_reading: None,
            brain_dirty: true,
            brain_prefer: None,
            brain_detail_generation: 0,
            brain_detail_reading: false,
            brain_detail_target: None,
            brain_detail_restore: None,
            brain_detail_back: false,
            brain_associations: None,
            brain_detail_claim: true,
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
        app.refresh_threads()?;
        app.refresh_selected()?;
        app.request_manual_history();
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
        if self.background_db.is_some() {
            return self.answer_memory_summaries.get(memory_id).cloned();
        }
        let insight = self.store.insight_for_memory(memory_id).ok().flatten()?;
        Some(super::db_reads::format_insight_summary(&insight))
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
        if first && self.background_db.is_none() {
            let _ = self.store.prune_events(super::logs::RETENTION_HOURS);
        }
        if self.module == Some(ModuleId::Intel) && self.intel_page == IntelPage::Briefing {
            self.refresh_intel_brief_stages();
        }
        match self.module {
            Some(ModuleId::Logs) => self.reload_logs(),
            Some(ModuleId::Jobs) => self.reload_jobs(),
            Some(ModuleId::System) => self.reload_profile(),
            _ => {
                if self.background_db.is_some() {
                    self.request_dashboard_snapshot();
                } else {
                    self.logs.refresh_counts(&self.store);
                }
            }
        }
        true
    }

    fn profile_analytics_visible(&self) -> bool {
        self.module == Some(ModuleId::System)
            && self.profile.tab == super::profile::SystemTab::Overview
            && self.overlay == Overlay::None
    }

    /// One visible Profile snapshot when its 20-second automatic refresh is due.
    /// The read is on the store's own connection, so it never blocks recording.
    pub(crate) fn reload_profile(&mut self) {
        if !self.profile_analytics_visible() || !self.profile.due() || self.profile.loading {
            return;
        }
        self.profile.loading = true;
        let generation = self.profile.generation;
        let filters = self.profile.filters.clone();
        let tx = self.work_tx.clone();
        let db = paths::db_path();
        std::thread::spawn(move || {
            let (outcome, options) = match Store::open_profile_reader(&db) {
                Ok(store) => (
                    store
                        .profile_snapshot(&filters.period, &filters)
                        .map(Box::new)
                        .map_err(|err| format!("{err:#}")),
                    store.profile_filter_options(),
                ),
                Err(err) => (Err(format!("{err:#}")), Vec::new()),
            };
            let _ = tx.send(WorkEvent::ProfileLoaded {
                generation,
                filters,
                outcome,
                options,
            });
        });
    }

    pub(crate) fn reload_logs(&mut self) {
        if self.background_db.is_some() {
            self.request_dashboard_snapshot();
            return;
        }
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
        if self.background_db.is_some() {
            self.request_dashboard_snapshot();
            return;
        }
        self.jobs.reload(&self.store);
        let room = super::jobs::table_room(super::ui::body_rect(self), &self.jobs);
        super::jobs::reveal(&mut self.jobs, room);
    }

    fn dashboard_read_key(&self) -> String {
        match self.module {
            Some(ModuleId::Jobs) => format!(
                "{:?}:{:?}:{:?}",
                self.jobs.filter(),
                self.jobs.selected().map(|j| &j.id),
                self.pending_job,
            ),
            Some(ModuleId::Logs) => format!("{:?}", self.logs.filter()),
            _ => String::new(),
        }
    }

    fn request_dashboard_snapshot(&mut self) {
        if self.dashboard_reading {
            return;
        }
        let Some(db) = self.background_db.clone() else {
            return;
        };
        self.dashboard_reading = true;
        let module = self.module;
        let key = self.dashboard_read_key();
        let mut logs = self.logs.clone();
        let mut jobs = self.jobs.clone();
        let pending_job = self.pending_job.clone();
        let tx = self.work_tx.clone();
        std::thread::spawn(move || {
            let outcome = Store::open_profile_reader(&db)
                .and_then(|store| {
                    store.connection().execute_batch("BEGIN")?;
                    Ok(Box::new(match module {
                        Some(ModuleId::Logs) => {
                            logs.reload(&store);
                            DashboardSnapshot::Logs(Box::new(logs))
                        }
                        Some(ModuleId::Jobs) => {
                            if let Some(id) = pending_job {
                                if !jobs.focus_job(&store, &id) {
                                    jobs.error = Some("Job history expired".into());
                                }
                            } else {
                                jobs.reload(&store);
                            }
                            DashboardSnapshot::Jobs(Box::new(jobs))
                        }
                        _ => DashboardSnapshot::Counts(store.event_counts()?),
                    }))
                })
                .map_err(|e| e.to_string());
            let _ = tx.send(WorkEvent::DashboardLoaded {
                module,
                key,
                outcome,
            });
        });
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
        if self.background_db.is_some() {
            self.pending_job = Some(id.into());
            self.select(ModuleId::Jobs.index());
            return Ok(format!("Loading job {}…", super::logs::short_id(id)));
        }
        self.select(ModuleId::Jobs.index());
        if self.jobs.focus_job(&self.store, id) {
            let room = super::jobs::table_room(super::ui::body_rect(self), &self.jobs);
            super::jobs::reveal(&mut self.jobs, room);
            self.set_focus(Target::JobRow(self.jobs.sel));
            Ok(format!("Job {}", super::logs::short_id(id)))
        } else {
            Ok(format!(
                "Job history expired · {}",
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
        self.pending_job = None;
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
        let mut items = super::commands::matching(&self.palette_query, self.module)
            .into_iter()
            .filter(|command| command.module.is_none() || command.module == self.module)
            .map(|command| PaletteItem {
                id: command.id.into(),
                label: command.label.into(),
                description: command.description.into(),
                shortcut: command.shortcut.into(),
                category: command.category.into(),
                enabled: match command.id {
                    "intel-visit" => self
                        .intel_articles
                        .get(self.intel_sel)
                        .is_some_and(|article| valid_browser_url(&article.url)),
                    "next-tab" | "prev-tab" | "close-tab" => !self.tab_ids.is_empty(),
                    "reopen-tab" => !self.tab_recently_closed.is_empty(),
                    "rename-investigation"
                    | "evidence"
                    | "thinking"
                    | "details"
                    | "trace"
                    | "directives"
                    | "jump-latest"
                        if !self.recon_chat =>
                    {
                        false
                    }
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
                    "intel-visit" => "Article URL must use HTTP or HTTPS".into(),
                    "cancel" => "No running turn".into(),
                    "resume" => "No interrupted or failed run to resume".into(),
                    "evidence" => "Select a completed evidence activity".into(),
                    _ => String::new(),
                },
            })
            .collect::<Vec<_>>();
        for item in &mut items {
            item.category = if self.palette_recent.contains(&item.id) {
                "Recently used"
            } else if super::commands::COMMANDS
                .iter()
                .find(|command| command.id == item.id)
                .is_some_and(|command| command.module.is_some())
            {
                "Current app shortcuts"
            } else {
                "Universal"
            }
            .into();
        }
        items.sort_by_key(|item| match item.category.as_str() {
            "Recently used" => (
                0,
                self.palette_recent
                    .iter()
                    .position(|id| id == &item.id)
                    .unwrap_or(8),
            ),
            "Current app shortcuts" => (1, 0),
            _ => (2, 0),
        });
        items
    }

    fn open_palette(&mut self) {
        if self.overlay == Overlay::Palette {
            self.overlay = Overlay::None;
            return;
        }
        self.palette_restore = Some(self.focus);
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
        if let Some(item) = self.palette_items().iter().find(|item| item.id == id) {
            if !item.enabled {
                self.status = item.disabled_reason.clone();
                return;
            }
        }
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
            "jobs-clear-completed" => self.activate_button(ButtonId::JobsClearCompleted),
            "jobs-clear-stopped" => self.activate_button(ButtonId::JobsClearStopped),
            "jobs-clear-failed" => self.activate_button(ButtonId::JobsClearFailed),
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
            "configs" => {
                self.select(ModuleId::System.index());
                super::profile::activate(self, Target::ProfileAction(3));
            }
            "intel-visit" => self.activate_button(ButtonId::IntelVisit),
            "previous-turn" => super::ui::navigate_turn(self, false),
            "next-turn" => super::ui::navigate_turn(self, true),
            "jump-latest" => {
                self.chat_follow = true;
                super::ui::normalize(self);
            }
            "intel-search" => self.set_focus(Target::Field(FieldId::IntelSearch)),

            "intel-extraction-retry" => self.activate_button(ButtonId::IntelExtractionRetry),
            "intel-extraction-job" => self.activate_button(ButtonId::IntelExtractionJob),
            "intel-refresh" => self.activate_button(ButtonId::IntelBodyRefresh),
            "intel-retry" => self.activate_button(ButtonId::IntelBodyRetry),
            "intel-job" => self.activate_button(ButtonId::IntelJobOpen),
            "thinking" | "details" | "trace" | "directives" => {
                let _ = self.run_slash(&format!("/{id}"));
            }
            _ => {}
        }
        if super::commands::COMMANDS
            .iter()
            .any(|command| command.id == id)
        {
            self.palette_recent.retain(|recent| recent != id);
            self.palette_recent.insert(0, id.into());
            self.palette_recent.truncate(8);
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

    pub(crate) fn config(&self) -> &super::profile_config::ConfigView {
        &self.profile_config
    }

    /// Draws the Configs export destination as a real field, so the mouse and
    /// the keyboard reach the same target.
    pub(crate) fn draw_export_field(&self, frame: &mut Frame, area: Rect) {
        super::ui::draw_field(frame, self, FieldId::ProfileExportPath, "destination", area);
    }

    /// Explicit Configs buttons keep verification separate from commit.
    fn config_action(&mut self, action: usize) {
        match action {
            0 => {
                if !self.profile_config.export_confirm {
                    self.profile_config.check_export_path();
                    if self.profile_config.export_confirm {
                        self.status = "Export again to confirm overwrite".into();
                        return;
                    }
                }
                if let Err(error) = self.profile_config.run_export() {
                    self.profile_config.export_error = Some(error.to_string());
                }
            }
            1 => self.profile_config.validate(),
            2 => match self.profile_config.run_import() {
                Ok(generation) => {
                    match (SettingsFile::load(), AuthFile::load()) {
                        (Ok(settings), Ok(auth)) => {
                            self.settings = settings;
                            self.auth = auth;
                            self.reload_configuration_fields();
                        }
                        _ => {
                            self.status = "Configuration saved; reload failed".into();
                            return;
                        }
                    }
                    self.status = format!("Applied configuration revision {generation}");
                }
                Err(error) => self.profile_config.import_error = Some(error.to_string()),
            },
            _ => {}
        }
    }

    fn reload_configuration_fields(&mut self) {
        if let Ok(secret) = provider::role_secret(&self.auth, &self.settings, "recon") {
            self.recon_provider = provider::effective_kind(&secret);
            self.recon_model = secret.model;
        }
        if let Ok(secret) = provider::role_secret(&self.auth, &self.settings, "tool-picker") {
            self.picker_provider = provider::effective_kind(&secret);
            self.picker_model = secret.model;
        }
        if let Ok(secret) = provider::role_secret(&self.auth, &self.settings, "synthesis") {
            self.synthesis_provider = provider::effective_kind(&secret);
            self.synthesis_model = secret.model;
        }
        if let Ok(secret) = provider::role_secret(&self.auth, &self.settings, "classifier") {
            self.classifier_provider = provider::effective_kind(&secret);
            self.classifier_model = secret.model;
        }
        if let Ok(secret) = provider::role_secret(&self.auth, &self.settings, "summarization") {
            self.summarization_provider = provider::effective_kind(&secret);
            self.summarization_model = secret.model;
        }
        if let Ok(secret) = provider::role_secret(&self.auth, &self.settings, "evidence_curator") {
            self.evidence_curator_provider = provider::effective_kind(&secret);
            self.evidence_curator_model = secret.model;
        }
        if let Ok(secret) = provider::role_secret(&self.auth, &self.settings, "entity_resolver") {
            self.entity_resolver_provider = provider::effective_kind(&secret);
            self.entity_resolver_model = secret.model;
        }
        if let Ok(secret) = provider::role_secret(&self.auth, &self.settings, "claim_assessor") {
            self.claim_assessor_provider = provider::effective_kind(&secret);
            self.claim_assessor_model = secret.model;
        }
        if let Ok(secret) =
            provider::role_secret(&self.auth, &self.settings, "investigation_controller")
        {
            self.investigation_controller_provider = provider::effective_kind(&secret);
            self.investigation_controller_model = secret.model;
        }
        let secret = provider::account_secret(&self.auth, "openrouter");
        self.router_key = secret.api_key.unwrap_or_default();
        self.router_endpoint = secret.base_url;
        let secret = provider::account_secret(&self.auth, "google");
        self.google_key = secret.api_key.unwrap_or_default();
        self.google_endpoint = secret.base_url;
        let secret = provider::account_secret(&self.auth, "nvidia");
        self.nvidia_key = secret.api_key.unwrap_or_default();
        self.nvidia_endpoint = secret.base_url;
        self.firecrawl_key = self.settings.provider_key("firecrawl");
        self.hunter_key = self.settings.provider_key("hunter");
        self.sociavault_key = self.settings.provider_key("sociavault");
        self.newsapi_key = self.settings.provider_key("newsapi");
        self.courtlistener_key = self.settings.provider_key("courtlistener");
        self.gnews_key = self.settings.provider_key("gnews");
        self.newsdata_key = self.settings.provider_key("newsdata");
        self.currents_key = self.settings.provider_key("currents");
        self.whoxy_key = self.settings.provider_key("whoxy");
        self.catalog_cache.clear();
        self.model_catalog.clear();
    }

    fn profile_config_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Tab | KeyCode::BackTab => {
                self.focus_next(key.code == KeyCode::BackTab);
                return true;
            }
            KeyCode::Esc => {
                self.profile_config.close();
                self.overlay = Overlay::None;
                self.profile.tab = super::profile::SystemTab::Overview;
                self.set_focus(Target::ProfileTab(2));
                return true;
            }
            KeyCode::Enter | KeyCode::Char(' ')
                if matches!(self.focus, Target::ConfigAction(_)) =>
            {
                if let Target::ConfigAction(action) = self.focus {
                    self.config_action(action);
                }
                return true;
            }
            _ => {}
        }
        if self.focus == Target::Field(FieldId::ProfileExportPath) {
            self.profile_config.export_confirm = false;
            return false;
        }
        if self.focus != Target::Field(FieldId::ProfileImportEditor) {
            return false;
        }
        let config = &mut self.profile_config;
        match key.code {
            KeyCode::Enter => config.newline(),
            KeyCode::Backspace => config.backspace(),
            KeyCode::Delete => config.delete(),
            KeyCode::Home => config.home(),
            KeyCode::End => config.end(),
            KeyCode::Left => config.import.caret = config.import.caret.saturating_sub(1),
            KeyCode::Right => {
                config.import.caret =
                    (config.import.caret + 1).min(config.import.text.chars().count())
            }
            KeyCode::Up => config.import.move_vertical(-1),
            KeyCode::PageUp => config.import.scroll_by(-8, 1),
            KeyCode::Down => config.import.move_vertical(1),
            KeyCode::PageDown => config.import.scroll_by(8, 1),
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                config.insert(&c.to_string())
            }
            _ => {}
        }
        true
    }

    pub(crate) fn select(&mut self, index: usize) {
        if ModuleId::ALL[index] != ModuleId::Jobs {
            self.pending_job = None;
        }
        self.brain_detail_target = None;
        self.brain_detail_generation = self.brain_detail_generation.wrapping_add(1);
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
            if self.background_db.is_some() {
                self.load_atlas();
            } else {
                self.atlas_runs = self.store.atlas_list_runs().unwrap_or_default();
            }
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
            Some(ModuleId::System) if self.profile.tab == super::profile::SystemTab::Overview => {
                Target::ProfileCard(self.profile.focus)
            }
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
        if let (Some(ModuleId::Brain), BrainListMode::Topology, Target::TopologyRelated(index)) =
            (self.module, self.brain_list_mode, target)
        {
            self.reveal_topology_related(index);
        }
        match target {
            Target::App(index) if self.module.is_none() => self.launcher_sel = index,
            Target::Button(ButtonId::FallbackItem(index)) => self.fallback_sel = index,
            _ => {}
        }
        super::ui::reveal_spaced_lists(self);
        if self.module == Some(ModuleId::Intel)
            && self.intel_page == IntelPage::Briefing
            && self.overlay == Overlay::None
        {
            match target {
                Target::IntelRelatedArticle(index) => {
                    self.intel_related.sel = index;
                    super::ui::reveal_intel_related(self, index);
                }
                Target::IntelTopologyClaim(_) | Target::IntelTopologyActor(_) => {
                    super::ui::reveal_intel_topology_entry(self, target);
                }
                Target::Button(ButtonId::RelatedScope | ButtonId::IntelRelatedMore) => {
                    let index = self.intel_related.sel;
                    super::ui::reveal_intel_related(self, index);
                }
                _ => {}
            }
        }
        if let Target::ProfileCard(index) = target {
            self.profile.focus = index;
            super::profile::reveal_focus(self);
        }
        if let Target::ProfileTable(index) = target {
            super::profile::reveal_table(self, index);
        }
        self.cursor = match target {
            Target::Field(field) => self.field(field).chars().count(),
            _ => 0,
        };
        self.sync_selected_insight();
    }

    fn sync_selected_insight(&mut self) {
        if self.background_db.is_some() && self.brain_list_mode == BrainListMode::List {
            self.brain_preview_target = if self.module == Some(ModuleId::Brain)
                && matches!(self.focus, Target::Memory(_))
            {
                self.memories
                    .get(self.memory_sel)
                    .filter(|m| !m.id.starts_with("actor-group:"))
                    .map(|m| m.id.clone())
            } else {
                None
            };
            if self.brain_preview_for == self.brain_preview_target {
                return;
            }
            self.selected_insight = None;
            self.request_brain_preview();
            return;
        }
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
        if let Some(db) = self.background_db.clone() {
            if self.threads_reading {
                self.threads_pending = true;
                return Ok(());
            }
            self.threads_reading = true;
            let query = self.recon_search.clone();
            let tx = self.work_tx.clone();
            std::thread::spawn(move || {
                let outcome = Store::open_profile_reader(&db)
                    .and_then(|store| {
                        store.connection().execute_batch("BEGIN")?;
                        let rows = store.list_threads(&query)?;
                        let states = rows
                            .iter()
                            .map(|t| {
                                Ok((
                                    t.id.clone(),
                                    store.latest_run_state(&t.id)?.unwrap_or_default(),
                                ))
                            })
                            .collect::<Result<_>>()?;
                        Ok((rows, states))
                    })
                    .map_err(|e| e.to_string());
                let _ = tx.send(WorkEvent::ThreadsLoaded { query, outcome });
            });
            return Ok(());
        }
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

    fn request_brain_preview(&mut self) {
        if self.brain_preview_reading {
            return;
        }
        let (Some(db), Some(id)) = (
            self.background_db.clone(),
            self.brain_preview_target.clone(),
        ) else {
            return;
        };
        self.brain_preview_reading = true;
        let generation = self.brain_generation;
        let tx = self.work_tx.clone();
        std::thread::spawn(move || {
            let outcome = Store::open_profile_reader(&db)
                .and_then(|store| store.insight_for_memory(&id))
                .map_err(|e| e.to_string());
            let _ = tx.send(WorkEvent::BrainPreviewLoaded {
                generation,
                memory_id: id,
                outcome,
            });
        });
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
            self.tool_expanded.insert(tool.category);
            self.response_scroll = 0;
            self.scrolls.detail = 0;
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
            let rows = super::tool_catalog::rows(&self.osint_search, &self.tool_expanded);
            if let Some(position) = rows
                .iter()
                .position(|(target, _)| *target == Target::Tool(index))
            {
                let room = super::ui::tool_room_for(self);
                super::ui::reveal_index(&mut self.scrolls.tools, position, room);
            }
            self.refresh_tool_dataset_status();
        }
    }

    fn refresh_tool_dataset_status(&mut self) {
        self.tool_dataset_status = match osint::registry().get(self.tool_sel).map(|tool| tool.id) {
            Some("whatsmyname_lookup") => {
                let status = osint::whatsmyname::status();
                if status.is_available {
                    format!(
                        "\nDataset: {} · {} supported sites · checked {}",
                        status.active_version.as_deref().unwrap_or("unknown"),
                        status.supported_count,
                        status.last_checked.as_deref().unwrap_or("unknown")
                    )
                } else {
                    "\nDataset not loaded · Refresh downloads the official catalog".into()
                }
            }
            Some("dork_generate") => {
                let status = osint::dork_generator::status();
                format!(
                    "\nTemplate catalog: {} · checked {}",
                    status.active_version.as_deref().unwrap_or("embedded seed"),
                    status.last_checked.as_deref().unwrap_or("embedded seed")
                )
            }
            _ => String::new(),
        };
    }

    pub(crate) fn open_profile_intel_job(&mut self, id: &str) -> Result<()> {
        let job = self
            .store
            .intel_report_job(id)?
            .ok_or_else(|| anyhow::anyhow!("Report job no longer retained"))?;
        let run_id: String = self.store.connection().query_row(
            "SELECT run_id FROM atlas_articles WHERE id=?1 ORDER BY seen_at DESC LIMIT 1",
            [&job.article_id],
            |row| row.get(0),
        )?;
        self.open_claim_article(&run_id, &job.article_id)?;
        self.status = format!("Report {} · {} · {}", job.id, job.state, job.stage);
        Ok(())
    }

    pub(crate) fn open_thread(&mut self, id: &str) -> Result<()> {
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
                self.store.save_draft(
                    previous,
                    &self.input,
                    i64::try_from(self.scrolls.chat).unwrap_or(i64::MAX),
                )?;
                self.draft_dirty = false;
            }
        }
        let thread = self
            .store
            .get_thread(id)?
            .ok_or_else(|| anyhow::anyhow!("Thread not found"))?;
        self.store.select_thread(id)?;
        self.selected_thread = Some(id.into());
        if self.background_db.is_none() {
            self.messages = self.store.list_messages(id)?;
        } else {
            self.messages.clear();
            self.calls.clear();
            self.investigation_events.clear();
            self.runs.clear();
            self.answer_memories.clear();
            self.answer_memory_summaries.clear();
        }
        self.refresh_selected()?;
        self.input = thread.draft;
        self.scrolls.chat = thread.scroll.max(0) as usize;
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
            self.store.save_draft(
                tid,
                "",
                i64::try_from(self.scrolls.chat).unwrap_or(i64::MAX),
            )?;
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

    pub fn can_stop_recon(&self) -> bool {
        self.selected_thread
            .as_ref()
            .is_some_and(|tid| self.running.contains_key(tid))
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
        if let Some(db) = self.background_db.clone() {
            if self.intel_list_reading {
                self.intel_list_pending = true;
                return;
            }
            self.intel_list_reading = true;
            if !INTEL_CATEGORIES.contains(&self.intel_category.as_str()) {
                self.intel_category = INTEL_CATEGORIES[0].into();
            }
            let key = (
                self.intel_category.clone(),
                self.intel_day.clone(),
                self.intel_search.clone(),
            );
            let tx = self.work_tx.clone();
            std::thread::spawn(move || {
                let outcome = Store::open_profile_reader(&db)
                    .and_then(|store| {
                        store.connection().execute_batch("BEGIN")?;
                        let days = store.atlas_run_days()?;
                        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
                        let day = if days.contains(&key.1) {
                            key.1.clone()
                        } else if days.contains(&today) {
                            today
                        } else {
                            days.first().cloned().unwrap_or_default()
                        };
                        let articles = if day.is_empty() {
                            Vec::new()
                        } else {
                            store.atlas_articles_for_intel(&key.0, &day, &key.2)?
                        };
                        Ok((days, day, articles))
                    })
                    .map_err(|e| e.to_string());
                let _ = tx.send(WorkEvent::IntelListLoaded { key, outcome });
            });
            return;
        }
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
        self.intel_related_limit = 20;
        let Some(article) = self.intel_articles.get(self.intel_sel).cloned() else {
            self.status = "No article selected".into();
            return;
        };
        self.intel_body = None;
        self.intel_brief_outputs.clear();
        self.intel_brief_stages.clear();
        self.intel_body_message.clear();
        self.intel_body_read_error = None;
        self.intel_body_owner = None;
        self.intel_body_require_commit = false;
        self.intel_brief_job = None;
        self.layout.borrow_mut().view_stamp.clear();
        self.intel_page = IntelPage::Briefing;
        self.intel_full_collapsed = false;
        self.scrolls.intel_brief = 0;
        self.scrolls.intel_full = 0;
        self.scrolls.intel_extracted = 0;
        // One idempotent preparation chain owns retrieval, cleanup, Claims,
        // Inferences and Links; the background brief runner executes it.
        // Mode classification is lazy (Recon configuration).
        match intel_recon::brief_stages::ensure_intel_brief_pipeline(
            &self.store,
            &article,
            &self.settings,
        ) {
            Ok(job) => self.intel_brief_job = Some(job),
            Err(error) => {
                self.intel_brief_job = None;
                self.intel_body_message = format!("Brief preparation unavailable: {error}");
            }
        }
        self.refresh_intel_briefing();
        self.refresh_intel_brief_stages();
        self.queue_intel_links_explanation();
        self.set_focus(Target::IntelPreview);
        self.status = format!("Brief · {}", article.title);
        self.persist_view_session();
    }

    fn retry_selected_extraction(&mut self) -> Result<String> {
        let Some(article) = self.intel_articles.get(self.intel_sel) else {
            return Ok("No article selected".into());
        };
        let Some(progress) = argos_osint_core::investigation::insight_execution::owner_progress(
            self.store.connection(),
            "article",
            &article.id,
        )?
        else {
            return Ok("No extraction task".into());
        };
        let retried = argos_osint_core::investigation::insight_execution::retry_extraction(
            self.store.connection(),
            &progress.task_id,
        )?;
        self.refresh_intel_briefing();
        Ok(if retried {
            "Retry extraction queued; completed work retained"
        } else {
            "No recoverable extraction work"
        }
        .into())
    }

    fn open_selected_extraction_job(&mut self) -> Result<String> {
        let Some(article) = self.intel_articles.get(self.intel_sel) else {
            return Ok("No article selected".into());
        };
        let Some(progress) = argos_osint_core::investigation::insight_execution::owner_progress(
            self.store.connection(),
            "article",
            &article.id,
        )?
        else {
            return Ok("No extraction job".into());
        };
        self.open_job(&progress.job_id)
    }

    /// Reload the shared stage views of the current brief chain; when a
    /// stage committed since the last poll, reload the briefing content.
    pub(crate) fn refresh_intel_brief_stages(&mut self) -> bool {
        if self.background_db.is_some() {
            self.refresh_intel_briefing();
            return false;
        }
        let previous = self.intel_brief_stages.clone();
        self.refresh_intel_briefing();
        previous != self.intel_brief_stages
    }

    fn refresh_intel_briefing(&mut self) {
        if let Some(db) = self.background_db.clone() {
            if self.intel_reading
                || self.module != Some(ModuleId::Intel)
                || self.intel_page != IntelPage::Briefing
            {
                return;
            }
            let Some(article) = self.intel_articles.get(self.intel_sel).cloned() else {
                return;
            };
            self.intel_reading = true;
            let job = self.intel_brief_job.clone();
            let scope = self.related_scope;
            let limit = self.intel_related_limit;
            let key = (
                article.run_id.clone(),
                article.id.clone(),
                scope,
                job.clone(),
                limit,
            );
            let tx = self.work_tx.clone();
            std::thread::spawn(move || {
                let outcome = Store::open_profile_reader(&db)
                    .and_then(|store| {
                        store.connection().execute_batch("BEGIN")?;
                        super::db_reads::IntelBriefSnapshot::read(
                            &store,
                            &article,
                            scope,
                            job.as_deref(),
                            limit,
                        )
                        .map(Box::new)
                    })
                    .map_err(|e| e.to_string());
                let _ = tx.send(WorkEvent::IntelBriefLoaded { key, outcome });
            });
            return;
        }
        let Some(article) = self.intel_articles.get(self.intel_sel).cloned() else {
            return;
        };
        let outcome = self
            .store
            .connection()
            .unchecked_transaction()
            .map_err(anyhow::Error::from)
            .and_then(|transaction| {
                let snapshot = super::db_reads::IntelBriefSnapshot::read(
                    &self.store,
                    &article,
                    self.related_scope,
                    self.intel_brief_job.as_deref(),
                    self.intel_related_limit,
                );
                transaction.rollback()?;
                snapshot
            });
        match outcome {
            Ok(snapshot) => self.accept_intel_brief_snapshot(snapshot),
            Err(error) => {
                self.intel_body_read_error = Some(error.to_string());
                self.layout.borrow_mut().view_stamp.clear();
            }
        }
    }

    fn accept_intel_brief_snapshot(&mut self, snapshot: super::db_reads::IntelBriefSnapshot) {
        let Some(article_id) = self
            .intel_articles
            .get(self.intel_sel)
            .map(|a| a.id.clone())
        else {
            return;
        };
        self.intel_actors = snapshot.actors;
        self.intel_actor_state = snapshot.actor_state;
        self.intel_metadata_present = snapshot.metadata_present;
        self.intel_publisher_prior = snapshot.publisher_prior;
        self.intel_extraction_progress = snapshot.progress;
        self.intel_insight_lifecycle = snapshot.lifecycle;
        self.intel_refresh_status = snapshot.status;
        self.intel_evidence_confidence = snapshot.confidence;
        let related_selected = self
            .intel_related
            .page
            .rows
            .get(self.intel_related.sel)
            .map(|r| (r.article.run_id.clone(), r.article.id.clone()));
        let old_scroll = self.intel_related.scroll;
        self.intel_related = snapshot.related;
        self.intel_related.sel = related_selected
            .and_then(|(run, id)| {
                self.intel_related
                    .page
                    .rows
                    .iter()
                    .position(|r| r.article.run_id == run && r.article.id == id)
            })
            .unwrap_or(0);
        self.intel_related.scroll = old_scroll;
        self.intel_claims = snapshot.claims;
        self.intel_relations = snapshot.relations;
        self.intel_link_explanations = snapshot.explanations;
        self.intel_body = snapshot.body;
        self.intel_body_require_commit = snapshot.body_requires_commit;
        self.intel_brief_stages = snapshot.stages;
        self.intel_brief_outputs = snapshot.outputs;
        self.intel_brief_lead_titles = snapshot.lead_titles;
        self.intel_brief_publication = snapshot.publication;
        if matches!(self.intel_refresh_status.as_str(), "Published" | "Indexed")
            || self.intel_refresh_status.starts_with("Previous")
        {
            self.intel_insights_running.remove(&article_id);
        }
        self.intel_body_owner = Some((article_id, self.intel_brief_job.clone()));
        self.intel_body_read_error = None;
        self.sync_intel_brief_task_ui();
        self.layout.borrow_mut().view_stamp.clear();
    }

    /// Reconcile in-memory loading flags/messages with store + background task sets on revisit.
    fn sync_intel_brief_task_ui(&mut self) {
        let Some(article) = self.intel_articles.get(self.intel_sel).cloned() else {
            return;
        };
        let ready = super::ui::intel_body_has_content(self);
        let active = super::ui::intel_body_loading(self);
        if ready || !active {
            self.intel_body_running.remove(&article.id);
            self.intel_body_message.clear();
        } else {
            self.intel_body_running.insert(article.id);
        }
    }

    fn leave_intel_briefing(&mut self) {
        if let Some(back) = self.related_back.pop() {
            match back {
                RelatedArticleBack::Article {
                    article,
                    related,
                    extracted_scroll,
                    brief_scroll,
                    full_scroll,
                    focus,
                } => match self.store.atlas_article(&article.run_id, &article.id) {
                    Ok(Some(current)) => {
                        let unchanged = argos_osint_core::atlas_metadata::input_hash(&current)
                            == argos_osint_core::atlas_metadata::input_hash(&article)
                            && current.category == article.category;
                        match self.open_intel_briefing_for(current) {
                            Ok(_) => {
                                if unchanged {
                                    self.intel_related = *related;
                                }
                                self.scrolls.intel_extracted = extracted_scroll;
                                self.scrolls.intel_brief = brief_scroll;
                                self.scrolls.intel_full = full_scroll;
                                self.set_focus(focus);
                            }
                            Err(error) => self.status = error.to_string(),
                        }
                    }
                    _ => {
                        self.leave_intel_briefing();
                        self.status = "Previous related article is unavailable".into();
                    }
                },
            }
            return;
        }
        self.intel_page = IntelPage::Bulletin;
        self.intel_claims.clear();
        self.intel_relations.clear();
        self.intel_body = None;
        // Keep body/insight/mode in-flight tracking and status text so revisit restores
        // the same loading UI instead of empty sections.
        self.set_focus(if self.intel_articles.is_empty() {
            Target::Field(FieldId::IntelSearch)
        } else {
            Target::IntelArticle(self.intel_sel)
        });
        self.status = "Bulletin".into();
    }

    fn queue_intel_links_explanation(&mut self) {
        // Stored assessed rationales were produced with the owner snapshot.
        // Viewing the brief never authorizes new investigative model work.
        if let Some(article) = self.intel_articles.get(self.intel_sel) {
            self.intel_link_explanations = self
                .store
                .get_intel_link_explanations(&article.id)
                .unwrap_or_default();
        }
    }

    pub fn move_intel(&mut self, delta: i32) {
        if self.intel_page != IntelPage::Bulletin || self.intel_articles.is_empty() {
            return;
        }
        let last = self.intel_articles.len() as i32 - 1;
        self.intel_preview_scroll = 0;
        self.intel_sel = (self.intel_sel as i32 + delta).clamp(0, last) as usize;
        self.set_focus(Target::IntelArticle(self.intel_sel));
        let room = super::ui::intel_list_room(self).max(1);
        super::ui::reveal_index(&mut self.scrolls.intel_list, self.intel_sel, room);
    }

    fn load_atlas(&mut self) {
        if let Some(db) = self.background_db.clone() {
            self.atlas_generation = self.atlas_generation.wrapping_add(1);
            if !self.atlas_reading {
                self.request_atlas_snapshot(db);
            }
            return;
        }
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

    fn request_atlas_snapshot(&mut self, db: PathBuf) {
        self.atlas_reading = true;
        let generation = self.atlas_generation;
        let tx = self.work_tx.clone();
        std::thread::spawn(move || {
            let outcome = Store::open_light(&db)
                .and_then(|store| {
                    let removed = store.atlas_prune_expired()?;
                    Ok((store.atlas_list_runs()?, removed))
                })
                .map_err(|e| e.to_string());
            let _ = tx.send(WorkEvent::AtlasLoaded {
                generation,
                outcome,
            });
        });
    }

    fn request_atlas_articles(&mut self) {
        if self.atlas_articles_reading {
            return;
        }
        let Some(db) = self.background_db.clone() else {
            return;
        };
        self.atlas_articles_reading = true;
        let run_id = self.atlas_news_run.clone();
        let tx = self.work_tx.clone();
        std::thread::spawn(move || {
            let outcome = Store::open_profile_reader(&db)
                .and_then(|store| store.atlas_list_articles(&run_id))
                .map_err(|e| e.to_string());
            let _ = tx.send(WorkEvent::AtlasArticlesLoaded { run_id, outcome });
        });
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
        if let Ok(Some(mut row)) = self.store.atlas_latest_article_by_id(&feed.id) {
            if !feed.category.trim().is_empty() {
                row.category = atlas::category_tag(&feed.category).into();
            }
            return row;
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
            .position(|row| row.id == article.id && row.run_id == article.run_id)
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
            .connection()
            .query_row(
                "SELECT substr(started_at,1,10) FROM atlas_runs WHERE id=?1",
                [run_id],
                |r| r.get::<_, String>(0),
            )
            .ok()
            .filter(|day| day.len() == 10)
    }

    fn delete_atlas_run(&mut self) -> Result<String> {
        let Some(run) = self.atlas_runs.get(self.atlas_run_sel).cloned() else {
            anyhow::bail!("No run selected");
        };
        if run.state == "running" {
            anyhow::bail!("Pause the pipeline before deleting this run");
        }
        if let Some(db) = self.background_db.clone() {
            anyhow::ensure!(
                self.atlas_deleting.is_none(),
                "A cycle deletion is already in progress"
            );
            self.atlas_deleting = Some(run.id.clone());
            let tx = self.work_tx.clone();
            std::thread::spawn(move || {
                let outcome = Store::open_light(&db)
                    .and_then(|store| store.atlas_delete_run(&run.id))
                    .map_err(|e| e.to_string());
                let _ = tx.send(WorkEvent::AtlasDeleted {
                    run_id: run.id,
                    outcome,
                });
            });
            return Ok("Deleting cycle…".into());
        }
        self.store.atlas_delete_run(&run.id)?;
        self.finish_atlas_delete(&run.id);
        Ok("Run deleted".into())
    }

    fn finish_atlas_delete(&mut self, run_id: &str) {
        self.reload_memories();
        if self.atlas_news_run == run_id {
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
    }

    fn open_atlas_news(&mut self) {
        let Some(run) = self.atlas_runs.get(self.atlas_run_sel) else {
            self.status = "No run selected".into();
            return;
        };
        let id = run.id.clone();
        if self.background_db.is_none() {
            self.atlas_articles = self.store.atlas_list_articles(&id).unwrap_or_default();
        } else {
            self.atlas_articles.clear();
        }
        self.atlas_news_run = id;
        self.atlas_news = true;
        self.claim_mark = None;
        self.atlas_article_sel = 0;
        self.request_atlas_articles();
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
    /// with retained collection/classification receipts) and nothing is running.
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
        let classifier = provider::role_secret(&self.auth, &self.settings, "classifier").ok();
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
                None,
                &[],
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
            WorkEvent::LogsCleared { outcome } => {
                self.logs_clearing = false;
                match outcome {
                    Ok(count) => {
                        self.logs.open.clear();
                        self.logs.scroll = 0;
                        self.logs.follow = true;
                        self.push_log("info", "Events cleared (jobs, results, and memories kept)");
                        self.status = format!("Cleared {count} events");
                        self.reload_logs();
                    }
                    Err(error) => self.status = format!("Could not clear events: {error}"),
                }
                true
            }
            WorkEvent::JobsCleared { outcome } => {
                self.jobs_clearing = false;
                self.jobs.scroll = 0;
                self.jobs.detail_scroll = 0;
                self.status =
                    outcome.unwrap_or_else(|error| format!("Could not clear jobs: {error}"));
                self.reload_jobs();
                self.refresh_intel_briefing();
                true
            }
            WorkEvent::BrainPreviewLoaded {
                generation,
                memory_id,
                outcome,
            } => {
                self.brain_preview_reading = false;
                if self.brain_list_mode != BrainListMode::List
                    || self.module != Some(ModuleId::Brain)
                {
                    return false;
                }
                if generation != self.brain_generation
                    || self.brain_preview_target.as_deref() != Some(&memory_id)
                {
                    self.request_brain_preview();
                    return true;
                }
                if let Ok(insight) = outcome {
                    self.selected_insight = insight;
                    self.brain_preview_for = Some(memory_id);
                }
                true
            }
            WorkEvent::ManualHistoryLoaded { outcome } => {
                self.manual_reading = false;
                if std::mem::take(&mut self.manual_pending) {
                    self.request_manual_history();
                    return true;
                }
                match outcome {
                    Ok(rows) => {
                        let selected = self
                            .manual_runs
                            .get(self.manual_run_pos)
                            .map(|c| c.id.clone());
                        self.manual_runs = rows;
                        self.manual_run_pos = selected
                            .and_then(|id| self.manual_runs.iter().position(|c| c.id == id))
                            .unwrap_or(self.manual_runs.len().saturating_sub(1));
                        if let Some(call) = self.manual_runs.get(self.manual_run_pos) {
                            self.osint_result = call.result.clone().map(|r| (call.id.clone(), r));
                        }
                    }
                    Err(error) => self.status = format!("Could not refresh tool history: {error}"),
                }
                true
            }
            WorkEvent::MemoryDeleted { memory_id, outcome } => {
                self.memories_deleting.remove(&memory_id);
                match outcome {
                    Ok(_) => {
                        if self.brain_detail.memory_id() == Some(memory_id.as_str()) {
                            self.leave_brain_detail();
                        }
                        self.brain_cache.retain(|m| m.id != memory_id);
                        self.filter_memory_cache();
                        self.reload_memories();
                        self.status = "Memory deleted".into();
                    }
                    Err(error) => self.status = format!("Could not delete memory: {error}"),
                }
                true
            }
            WorkEvent::ReconLoaded { thread_id, outcome } => {
                self.recon_reading = false;
                // Logging already committed on the worker, even if its view
                // snapshot is superseded. Retain those IDs to avoid replay.
                if let Ok(snapshot) = &outcome {
                    self.logged_calls
                        .extend(snapshot.logged_calls.iter().cloned());
                }
                if std::mem::take(&mut self.recon_pending) {
                    let _ = self.refresh_selected();
                    return true;
                }
                if self.selected_thread.as_deref() != Some(&thread_id) {
                    let _ = self.refresh_selected();
                    return true;
                }
                match outcome {
                    Ok(snapshot) => {
                        self.messages = snapshot.messages;
                        self.investigation_events = snapshot.events;
                        self.calls = snapshot.calls;
                        self.runs = snapshot.runs;
                        self.answer_memories = snapshot.memories;
                        self.answer_memory_summaries = snapshot.memory_summaries;
                        if let Some(error) = snapshot.log_error {
                            self.logs.error =
                                Some(format!("Could not record tool history: {error}"));
                        }
                        self.note_finished_calls();
                    }
                    Err(error) => self.status = format!("Could not refresh investigation: {error}"),
                }
                true
            }
            WorkEvent::ThreadsLoaded { query, outcome } => {
                self.threads_reading = false;
                if std::mem::take(&mut self.threads_pending) {
                    let _ = self.refresh_threads();
                    return true;
                }
                if query != self.recon_search {
                    let _ = self.refresh_threads();
                    return true;
                }
                match outcome {
                    Ok((rows, states)) => {
                        let selected = self.threads.get(self.thread_sel).map(|t| t.id.clone());
                        self.threads = rows;
                        self.thread_states = states;
                        self.thread_sel = selected
                            .and_then(|id| self.threads.iter().position(|t| t.id == id))
                            .unwrap_or(self.thread_sel.min(self.threads.len().saturating_sub(1)));
                        if matches!(self.focus, Target::Thread(_)) {
                            self.focus = if self.threads.is_empty() {
                                Target::Field(FieldId::ReconSearch)
                            } else {
                                Target::Thread(self.thread_sel)
                            };
                        }
                    }
                    Err(error) => {
                        self.status = format!("Could not refresh investigations: {error}")
                    }
                }
                true
            }
            WorkEvent::IntelListLoaded { key, outcome } => {
                self.intel_list_reading = false;
                if std::mem::take(&mut self.intel_list_pending) {
                    self.load_intel();
                    return true;
                }
                if key
                    != (
                        self.intel_category.clone(),
                        self.intel_day.clone(),
                        self.intel_search.clone(),
                    )
                {
                    self.load_intel();
                    return true;
                }
                match outcome {
                    Ok((days, day, mut rows)) => {
                        let selected = self.intel_articles.get(self.intel_sel).cloned();
                        if self.intel_page == IntelPage::Briefing {
                            if let Some(article) = &selected {
                                if !rows
                                    .iter()
                                    .any(|a| a.id == article.id && a.run_id == article.run_id)
                                {
                                    rows.insert(0, article.clone());
                                }
                            }
                        }
                        self.intel_days = days;
                        self.intel_day = day;
                        self.intel_articles = rows;
                        self.intel_sel = selected
                            .and_then(|selected| {
                                self.intel_articles.iter().position(|a| {
                                    a.id == selected.id && a.run_id == selected.run_id
                                })
                            })
                            .unwrap_or(
                                self.intel_sel
                                    .min(self.intel_articles.len().saturating_sub(1)),
                            );
                        if matches!(self.focus, Target::IntelArticle(_)) {
                            self.focus = if self.intel_articles.is_empty() {
                                Target::Field(FieldId::IntelSearch)
                            } else {
                                Target::IntelArticle(self.intel_sel)
                            };
                        }
                        if self.intel_page == IntelPage::Briefing {
                            self.refresh_intel_briefing();
                        }
                    }
                    Err(error) => self.status = format!("Could not refresh bulletin: {error}"),
                }
                true
            }
            WorkEvent::AtlasArticlesLoaded { run_id, outcome } => {
                self.atlas_articles_reading = false;
                if !self.atlas_news {
                    return false;
                }
                if run_id != self.atlas_news_run {
                    self.request_atlas_articles();
                    return true;
                }
                match outcome {
                    Ok(rows) => {
                        self.atlas_articles = rows;
                        self.atlas_article_sel = self
                            .atlas_article_sel
                            .min(self.atlas_articles.len().saturating_sub(1));
                    }
                    Err(error) => {
                        self.status = format!("Could not refresh cycle articles: {error}")
                    }
                }
                true
            }
            WorkEvent::ThreadDeleted { thread_id, outcome } => {
                self.threads_deleting.remove(&thread_id);
                match outcome {
                    Ok(removed) => {
                        self.finish_thread_delete(&thread_id);
                        self.status =
                            format!("Investigation deleted; {removed} brain memories removed");
                    }
                    Err(error) => self.status = format!("Could not delete investigation: {error}"),
                }
                true
            }
            WorkEvent::DashboardLoaded {
                module,
                key,
                outcome,
            } => {
                self.dashboard_reading = false;
                if module != self.module || key != self.dashboard_read_key() {
                    self.request_dashboard_snapshot();
                    return true;
                }
                match outcome {
                    Ok(snapshot) => match *snapshot {
                        DashboardSnapshot::Logs(snapshot) => {
                            self.logs.accept(*snapshot);
                            let (room, width) =
                                super::logs::list_geometry(super::ui::body_rect(self));
                            self.logs.reveal(room, width);
                            let max = self.logs.scroll_max(room, width);
                            self.logs.scroll = if self.logs.follow {
                                max
                            } else {
                                self.logs.scroll.min(max)
                            };
                        }
                        DashboardSnapshot::Jobs(snapshot) => {
                            let select_snapshot = self.pending_job.take().is_some();
                            if select_snapshot {
                                self.jobs.status = snapshot.status;
                                self.jobs.app = snapshot.app.clone();
                                self.jobs.search = snapshot.search.clone();
                            }
                            self.jobs.accept(*snapshot, select_snapshot);
                            if matches!(self.focus, Target::JobRow(_)) {
                                self.focus = Target::JobRow(self.jobs.sel);
                            }
                            let room =
                                super::jobs::table_room(super::ui::body_rect(self), &self.jobs);
                            super::jobs::reveal(&mut self.jobs, room);
                        }
                        DashboardSnapshot::Counts(counts) => self.logs.counts = counts,
                    },
                    Err(error) => match self.module {
                        Some(ModuleId::Logs) => self.logs.error = Some(error),
                        Some(ModuleId::Jobs) => self.jobs.error = Some(error),
                        _ => {}
                    },
                }
                true
            }
            WorkEvent::IntelBriefLoaded { key, outcome } => {
                self.intel_reading = false;
                let current = self.intel_articles.get(self.intel_sel).map(|a| {
                    (
                        a.run_id.clone(),
                        a.id.clone(),
                        self.related_scope,
                        self.intel_brief_job.clone(),
                        self.intel_related_limit,
                    )
                });
                if self.module != Some(ModuleId::Intel) || self.intel_page != IntelPage::Briefing {
                    return false;
                }
                if current.as_ref() != Some(&key) {
                    self.refresh_intel_briefing();
                    return true;
                }
                match outcome {
                    Ok(snapshot) => {
                        self.accept_intel_brief_snapshot(*snapshot);
                    }
                    Err(error) => {
                        self.intel_body_read_error = Some(error);
                        self.layout.borrow_mut().view_stamp.clear();
                    }
                }
                true
            }
            WorkEvent::AtlasLoaded {
                generation,
                outcome,
            } => {
                self.atlas_reading = false;
                if generation != self.atlas_generation {
                    if let Some(db) = self.background_db.clone() {
                        self.request_atlas_snapshot(db);
                    }
                    return true;
                }
                match outcome {
                    Ok((runs, removed)) => {
                        let selected = self
                            .atlas_runs
                            .get(self.atlas_run_sel)
                            .map(|r| r.id.clone());
                        let news_run = self.atlas_news_run.clone();
                        self.atlas_runs = runs;
                        self.atlas_run_sel = selected
                            .as_ref()
                            .and_then(|id| self.atlas_runs.iter().position(|r| &r.id == id))
                            .unwrap_or(
                                self.atlas_run_sel
                                    .min(self.atlas_runs.len().saturating_sub(1)),
                            );
                        if self.atlas_pause.is_none() {
                            if let Some(run) = self.atlas_runs.first() {
                                self.atlas_stats =
                                    serde_json::from_str(&run.stats_json).unwrap_or_default();
                                self.atlas_state = run.state.clone();
                            } else {
                                self.atlas_stats = Default::default();
                                self.atlas_state = "idle".into();
                            }
                        }
                        self.dismiss_pruned_atlas(&removed, selected.as_deref(), &news_run);
                        if !removed.is_empty() {
                            self.reload_memories();
                        }
                    }
                    Err(error) => {
                        self.push_log_detail("error", "Atlas: could not refresh history", error)
                    }
                }
                true
            }
            WorkEvent::AtlasDeleted { run_id, outcome } => {
                if self.atlas_deleting.as_deref() != Some(&run_id) {
                    return false;
                }
                self.atlas_deleting = None;
                match outcome {
                    Ok(_) => {
                        self.finish_atlas_delete(&run_id);
                        self.status = "Run deleted".into();
                    }
                    Err(error) => {
                        self.status = format!("Could not delete cycle: {error}");
                        self.push_log_detail("error", "Atlas: cycle deletion failed", error);
                    }
                }
                true
            }
            WorkEvent::BrainDetailLoaded {
                generation,
                outcome,
            } => {
                self.on_brain_detail_loaded(generation, outcome);
                true
            }
            WorkEvent::TopologyLoaded {
                generation,
                seed,
                outcome,
            } => {
                self.topology_reading = false;
                if generation != self.topology_generation
                    || self.topology_state.seed.as_ref() != Some(&seed)
                {
                    if self.module == Some(ModuleId::Brain)
                        && self.brain_list_mode == BrainListMode::Topology
                    {
                        self.refresh_topology();
                    }
                    return true;
                }
                if generation == self.topology_generation
                    && self.topology_state.seed.as_ref() == Some(&seed)
                {
                    let visible = self.module == Some(ModuleId::Brain)
                        && self.brain_list_mode == BrainListMode::Topology;
                    self.topology_loading = false;
                    match outcome {
                        Ok(snap) => {
                            let focus = self.topology_state.pending_focus.take().or_else(|| {
                                super::topology::stable_focus(&self.topology_state, self.focus)
                            });
                            if !snap.nodes.iter().any(|node| {
                                Some(&node.id) == self.topology_state.selected_node_id.as_ref()
                            }) {
                                self.topology_state.selected_node_id =
                                    (!snap.root_id.is_empty()).then(|| snap.root_id.clone());
                            }
                            if !snap.edges.iter().any(|edge| {
                                Some(&edge.id) == self.topology_state.selected_edge_id.as_ref()
                            }) {
                                self.topology_state.selected_edge_id = None;
                            }
                            self.topology_state.snapshot = Some(*snap);
                            self.topology_error = None;
                            self.layout.borrow_mut().view_stamp.clear();
                            if visible {
                                if let Some(target) = focus.and_then(|focus| {
                                    super::topology::resolve_focus(&self.topology_state, focus)
                                }) {
                                    self.set_focus(target);
                                }
                                self.status = "Topology mapper".into();
                            }
                        }
                        Err(err) => {
                            self.topology_error = Some(err.clone());
                            if visible {
                                self.status = format!("Topology load error: {err}");
                            }
                            self.push_log_detail("error", "Brain: topology snapshot failed", err);
                        }
                    }
                }
                true
            }
            WorkEvent::BrainLoaded {
                generation,
                outcome,
            } => {
                self.on_brain_loaded(generation, outcome);
                true
            }
            WorkEvent::BrainMaintenance { outcome } => {
                if let Err(error) = outcome {
                    self.push_log_detail("warn", "Brain: background repair incomplete", error);
                }
                self.reload_memories();
                true
            }
            WorkEvent::ProfileLoaded {
                generation,
                filters,
                outcome,
                options,
            } => {
                self.profile.loading = false;
                // A read may finish after navigation. Retain the previous snapshot
                // and its timestamp so returning to overdue analytics retries.
                if !self.profile_analytics_visible() {
                    return false;
                }
                if generation != self.profile.generation || filters != self.profile.filters {
                    return true;
                }
                self.profile.loaded_at = Some(Instant::now());
                self.profile.options = options;
                match outcome {
                    Ok(snapshot) => {
                        self.profile.snapshot_filters = Some(filters);
                        self.profile.accept_snapshot(*snapshot);
                    }
                    Err(error) => self.profile.error = Some(error),
                }
                true
            }
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
                        if self.background_db.is_some() {
                            self.request_manual_history();
                        } else if let Ok(runs) = self.store.manual_calls() {
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
                self.refresh_tool_dataset_status();
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
        // Same contract rule as config import (spec §5): bounded-decision
        // and mixed roles accept native decisions models; only the
        // generation-only roles need every added route to complete chat.
        if !matches!(
            self.defaults_role,
            DefaultsRole::Recon | DefaultsRole::Synthesis | DefaultsRole::Summarization
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

    /// Apply the Brief fast preset, or reset it when applied. Reset removes
    /// only the preset's references; Summarization itself is never changed.
    pub fn toggle_fast_preset(&mut self) -> Result<String> {
        let applied = self.settings.defaults.fast_preset_applied();
        if applied {
            self.settings.defaults.reset_fast_preset();
        } else {
            self.settings.defaults.apply_fast_preset();
        }
        self.save_settings()?;
        Ok(if applied {
            "Brief fast preset reset".into()
        } else {
            "Brief fast preset applied: Claims and Links follow Summarization".into()
        })
    }

    /// Selected Brief operation `(key, label, role)`.
    pub fn selected_brief_op(&self) -> (&'static str, &'static str, &'static str) {
        provider::BRIEF_OPERATIONS[self.brief_op_sel % provider::BRIEF_OPERATIONS.len()]
    }

    pub fn cycle_brief_op(&mut self) -> String {
        self.brief_op_sel = (self.brief_op_sel + 1) % provider::BRIEF_OPERATIONS.len();
        format!("Editing Brief operation: {}", self.selected_brief_op().1)
    }

    /// Save the current Provider/Model fields as an explicit override for
    /// the selected Brief operation. Role semantics and scope never change.
    pub fn save_brief_op_override(&mut self) -> Result<String> {
        let role = self.defaults_role;
        let provider = provider::normalize_kind(self.field(role.provider_field()).trim());
        let model = self.field(role.model_field()).trim().to_string();
        if provider.is_empty() || model.is_empty() {
            anyhow::bail!("Provider and model are required");
        }
        let (key, label, _) = self.selected_brief_op();
        self.settings.defaults.operation_overrides.insert(
            key.to_string(),
            provider::ModelAssignment {
                provider: provider.clone(),
                model: model.clone(),
                ..Default::default()
            }
            .into(),
        );
        self.save_settings()?;
        Ok(format!("{label} override saved: {provider} / {model}"))
    }

    /// Reset the selected operation: it inherits its logical role again.
    pub fn reset_brief_op_override(&mut self) -> Result<String> {
        let (key, label, role) = self.selected_brief_op();
        if self
            .settings
            .defaults
            .operation_overrides
            .remove(key)
            .is_none()
        {
            return Ok(format!("{label} already inherits {role}"));
        }
        self.save_settings()?;
        Ok(format!("{label} override reset: inherits {role}"))
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
        if let Some(db) = self.background_db.clone() {
            if self.recon_reading {
                self.recon_pending = true;
                return Ok(());
            }
            let Some(id) = self.selected_thread.clone() else {
                return Ok(());
            };
            self.recon_reading = true;
            let logged_calls = self.logged_calls.clone();
            let tx = self.work_tx.clone();
            std::thread::spawn(move || {
                let outcome = Store::open_profile_reader(&db)
                    .and_then(|store| {
                        store.connection().execute_batch("BEGIN")?;
                        super::db_reads::ReconSnapshot::read(&store, &id)
                    })
                    .map(|mut snapshot| {
                        match Store::open_light(&db) {
                            Ok(store) => snapshot.record_call_logs(&store, &logged_calls),
                            Err(error) => snapshot.log_error = Some(error.to_string()),
                        }
                        Box::new(snapshot)
                    })
                    .map_err(|e| e.to_string());
                let _ = tx.send(WorkEvent::ReconLoaded {
                    thread_id: id,
                    outcome,
                });
            });
            return Ok(());
        }
        if let Some(tid) = &self.selected_thread {
            self.messages = self.store.list_messages(tid)?;
            self.investigation_events = self.store.list_investigation_events(tid)?;
            self.calls = self.store.all_calls_for_thread(tid)?;
            self.runs = self.store.runs_for_thread(tid)?;
            self.answer_memories = self.store.answer_memories(tid)?;
            self.note_finished_calls();
        }
        Ok(())
    }

    fn request_manual_history(&mut self) {
        if self.manual_reading {
            self.manual_pending = true;
            return;
        }
        let Some(db) = self.background_db.clone() else {
            return;
        };
        self.manual_reading = true;
        let tx = self.work_tx.clone();
        std::thread::spawn(move || {
            let outcome = Store::open_profile_reader(&db)
                .and_then(|store| store.manual_calls())
                .map_err(|e| e.to_string());
            let _ = tx.send(WorkEvent::ManualHistoryLoaded { outcome });
        });
    }

    /// Appends one collapsible System log entry the first time a call has a result.
    fn note_finished_calls(&mut self) {
        if self.background_db.is_some() {
            return;
        }
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
        let memory = self.store.add_user_claim(
            &text,
            category,
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
        // List rows do not display graphs. Production reads belong to detail
        // loading, so arrow keys and refresh completion never query a graph.
        if self.background_db.is_some() && self.brain_list_mode == BrainListMode::List {
            return;
        }
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
        if self
            .brain_detail
            .memory_id()
            .is_some_and(|id| id.starts_with("actor-group:"))
        {
            return false;
        }
        if self.background_db.is_some() {
            return self.brain_detail_claim;
        }
        // Typed source-claim/inference semantics win over placement heuristics.
        if let Some(kind) = self
            .brain_detail
            .memory
            .as_ref()
            .and_then(|memory| self.store.memory_insight_kind(&memory.id))
        {
            return kind == "claim";
        }
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
        let list = self
            .store
            .brain_browse_memories(self.brain_kind, self.brain_query.trim())?;
        let total = self.store.brain_browse(self.brain_kind, "")?.len();
        Ok((list, total))
    }

    /// Switch Related between all retained cycles and the source cycle and
    /// reload the open Intel article and Brain memory Related lists.
    pub(crate) fn toggle_related_scope(&mut self) {
        self.related_scope = self.related_scope.toggle();
        if self.background_db.is_some() {
            self.refresh_intel_briefing();
            self.status = format!("Related: {}", self.related_scope.label());
            return;
        }
        if let Some(article) = self.intel_articles.get(self.intel_sel).cloned() {
            match self.store.related_articles_global(
                &article.run_id,
                &article.id,
                self.related_scope,
                None,
                20,
            ) {
                Ok(page) => {
                    self.intel_related.page = page.into_page(0);
                    self.intel_related.error = None;
                }
                Err(error) => self.intel_related.error = Some(error.to_string()),
            }
            self.intel_related.sel = 0;
            self.intel_related.scroll = 0;
        }
        self.status = format!("Related: {}", self.related_scope.label());
    }

    /// Next Brain kind filter: All, then each of the five core kinds.
    pub(crate) fn cycle_brain_kind(&mut self) {
        use argos_osint_core::memory_metadata::MemoryKind;
        let order = [MemoryKind::Claim, MemoryKind::Actor];
        self.brain_kind = match self.brain_kind {
            None => Some(order[0]),
            Some(kind) => order
                .iter()
                .position(|k| *k == kind)
                .and_then(|i| order.get(i + 1).copied()),
        };
        self.memory_sel = 0;
        self.scrolls.memories = 0;
        self.filter_memory_cache();
        self.status = format!("Brain kind: {}", brain_kind_label(self.brain_kind));
    }

    pub(crate) fn reload_memories(&mut self) {
        self.reload_memories_selecting(None);
    }

    /// Reload the Brain list, keeping the selected memory (by id), Find, and
    /// scroll. A read failure keeps the last good list and reports the error.
    fn reload_memories_selecting(&mut self, prefer: Option<String>) {
        if self.background_db.is_some() {
            if prefer.is_some() {
                self.brain_prefer = prefer;
            }
            self.brain_generation = self.brain_generation.wrapping_add(1);
            self.brain_dirty = true;
            self.request_memory_snapshot();
            return;
        }
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
        self.restore_memory_selection(selected);
    }

    fn request_memory_snapshot(&mut self) {
        if self.module != Some(ModuleId::Brain) || !self.brain_dirty || self.brain_reading.is_some()
        {
            return;
        }
        let Some(db) = self.background_db.clone() else {
            return;
        };
        let generation = self.brain_generation;
        self.brain_reading = Some(generation);
        let tx = self.work_tx.clone();
        std::thread::spawn(move || {
            let outcome = Store::open_profile_reader(&db)
                .and_then(|store| {
                    // One consistent read transaction; concurrent ingest cannot
                    // produce mixed revisions in the display projection.
                    store.connection().execute_batch("BEGIN")?;
                    store.brain_browse_memories(None, "")
                })
                .map_err(|e| e.to_string());
            let _ = tx.send(WorkEvent::BrainLoaded {
                generation,
                outcome,
            });
        });
    }

    fn on_brain_loaded(&mut self, generation: u64, outcome: Result<Vec<Memory>, String>) {
        if self.brain_reading != Some(generation) {
            return;
        }
        self.brain_reading = None;
        if generation != self.brain_generation {
            self.request_memory_snapshot();
            return;
        }
        self.brain_dirty = false;
        match outcome {
            Ok(list) => {
                self.brain_cache = list;
                self.brain_preview_for = None;
                self.memories_loaded = true;
                if self.memory_error.take().is_some() {
                    self.push_log("info", "Brain: memories readable again");
                }
                self.filter_memory_cache();
            }
            Err(message) => {
                if self.memory_error.as_deref() != Some(&message) {
                    self.push_log_detail(
                        "error",
                        "Brain: could not read memories",
                        message.clone(),
                    );
                }
                self.memory_error = Some(message);
                self.status = "Could not read memories; showing the last loaded list".into();
            }
        }
    }

    fn filter_memory_cache(&mut self) {
        if self.background_db.is_none() {
            self.reload_memories();
            return;
        }
        let preferred = if self.brain_dirty {
            self.brain_prefer.clone()
        } else {
            self.brain_prefer.take()
        };
        let selected =
            preferred.or_else(|| self.memories.get(self.memory_sel).map(|m| m.id.clone()));
        let query = self.brain_query.trim().to_lowercase();
        let eligible = |m: &&Memory| match self.brain_kind {
            None => true,
            Some(argos_osint_core::memory_metadata::MemoryKind::Actor) => {
                m.id.starts_with("actor-group:")
            }
            Some(argos_osint_core::memory_metadata::MemoryKind::Claim) => {
                !m.id.starts_with("actor-group:")
            }
            _ => false,
        };
        self.memory_total = self.brain_cache.iter().filter(eligible).count();
        self.memories = self
            .brain_cache
            .iter()
            .filter(eligible)
            .filter(|m| m.text.to_lowercase().contains(&query))
            .cloned()
            .collect();
        self.restore_memory_selection(selected);
    }

    fn restore_memory_selection(&mut self, selected: Option<String>) {
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
        if self.background_db.is_some() {
            // A database refresh must not replace an in-flight destination
            // chosen by the user with the record still visible underneath it.
            if self.brain_detail_target.is_some() {
                return;
            }
            self.request_brain_detail(&id);
            self.brain_detail_restore = self.detail_snapshot();
            return;
        }
        if id.starts_with("actor-group:") {
            return;
        }
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
        self.brain_detail_target = None;
        self.brain_detail_generation = self.brain_detail_generation.wrapping_add(1);
        self.brain_associations = None;
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
            compact_page: self.compact_pages[ModuleId::Brain.index()],
            related_sel: self.brain_detail.related.sel,
            related_scroll: self.brain_detail.related.scroll,
            tab: self.brain_detail.related.tab,
            cursors: self.brain_detail.related.cursors,
            page_sizes: self.brain_detail.related.page_sizes,
            selected_keys: {
                let mut keys = self.brain_detail.related.selected_keys.clone();
                keys[self.brain_detail.related.tab.index()] = self
                    .brain_detail
                    .selected_related()
                    .map(|item| item.key.clone());
                keys
            },
            path_scroll: self.scrolls.path,
            summary_scroll: self.scrolls.summary,
            focus: self.focus,
        })
    }

    pub(crate) fn open_topology(&mut self, seed: argos_osint_core::brain_topology::TopologySeed) {
        let already_open =
            self.module == Some(ModuleId::Brain) && self.brain_list_mode == BrainListMode::Topology;
        if already_open {
            let focus = super::topology::stable_focus(&self.topology_state, self.focus);
            self.topology_state.push_history();
            if let Some(frame) = self.topology_state.history.last_mut() {
                frame.focus = focus;
            }
        } else {
            self.topology_origin_label = if self.module == Some(ModuleId::Intel) {
                "Intel · Focus brief".into()
            } else if self
                .brain_detail
                .memory_id()
                .is_some_and(|id| id.starts_with("actor-group:"))
            {
                "Brain · actor group graph".into()
            } else {
                "Brain · claim path".into()
            };
            let origin_article = (self.module == Some(ModuleId::Intel))
                .then(|| self.intel_articles.get(self.intel_sel))
                .flatten();
            self.topology_origin = Some(TopologyOrigin {
                module: self.module,
                brain_mode: self.brain_list_mode,
                intel_page: self.intel_page,
                article_id: origin_article.map(|a| a.id.clone()),
                compact_pages: self.compact_pages,
                scrolls: self.scrolls.clone(),
                focus: self.focus,
                article_revision: origin_article.map(|a| {
                    (
                        a.run_id.clone(),
                        argos_osint_core::atlas_metadata::input_hash(a),
                    )
                }),
            });
            self.topology_state = Default::default();
        }
        self.module = Some(ModuleId::Brain);
        self.brain_list_mode = BrainListMode::Topology;
        self.topology_state.seed = Some(seed);
        self.topology_state.snapshot = None;
        self.topology_state.selected_node_id = None;
        self.topology_state.selected_edge_id = None;
        self.topology_state.pending_focus = None;
        self.refresh_topology();
        self.set_focus(Target::Button(ButtonId::TopologyBack));
    }

    pub(crate) fn refresh_topology(&mut self) {
        if let Some(seed) = self.topology_state.seed.clone() {
            self.topology_generation = self.topology_generation.wrapping_add(1);
            let generation = self.topology_generation;
            self.topology_loading = true;
            self.topology_error = None;
            self.status = if self.topology_state.snapshot.is_some() {
                "Refreshing topology · previous snapshot retained".into()
            } else {
                "Loading topology…".into()
            };
            if self.topology_reading {
                return;
            }
            let options = argos_osint_core::brain_topology::TopologyOptions {
                depth: self.topology_state.depth,
                cycle_scope: self.topology_state.cycle_scope.clone(),
                max_nodes: 100,
                max_edges: 200,
                continuation_token: None,
            };
            if let Some(db) = self.background_db.clone() {
                self.topology_reading = true;
                let tx = self.work_tx.clone();
                std::thread::spawn(move || {
                    let outcome = Store::open_light(&db)
                        .and_then(|store| store.topology_snapshot(&seed, &options).map(Box::new))
                        .map_err(|e| e.to_string());
                    let _ = tx.send(WorkEvent::TopologyLoaded {
                        generation,
                        seed,
                        outcome,
                    });
                });
            } else {
                // In-memory fixtures have no independent connection to dispatch.
                let outcome = self
                    .store
                    .topology_snapshot(&seed, &options)
                    .map(Box::new)
                    .map_err(|e| e.to_string());
                self.on_work_event(WorkEvent::TopologyLoaded {
                    generation,
                    seed,
                    outcome,
                });
            }
        }
    }

    pub(crate) fn intel_topology_claim_entries(&self) -> Vec<(String, String)> {
        if let Some(output) = self.intel_brief_outputs.get("intel_brief_claims") {
            return output["claims"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|claim| {
                    let id = claim["id"].as_str()?;
                    let text = claim["text"].as_str()?;
                    let who = claim["attributed_to"].as_str().unwrap_or("");
                    let modality = claim["modality"].as_str().unwrap_or("");
                    let head = [who, modality]
                        .into_iter()
                        .filter(|s| !s.is_empty())
                        .collect::<Vec<_>>()
                        .join(" · ");
                    Some((
                        id.to_string(),
                        if head.is_empty() {
                            text.to_string()
                        } else {
                            format!("{head}: {text}")
                        },
                    ))
                })
                .collect();
        }
        self.intel_claims
            .iter()
            .filter(|c| c.classification != "inference")
            .map(|c| {
                (
                    c.fingerprint.clone(),
                    if c.fingerprint.starts_with("v2:") {
                        c.claim.clone()
                    } else {
                        format!("{} ({:.2})", c.claim, c.confidence)
                    },
                )
            })
            .collect()
    }

    pub(crate) fn intel_topology_actor_entries(&self) -> Vec<(String, String)> {
        let mut entries: Vec<_> = self
            .intel_actors
            .iter()
            .map(|actor| (actor.id.clone(), actor.display_name.clone()))
            .collect();
        for claim in &self.intel_claims {
            if !claim.entity.trim().is_empty()
                && !entries
                    .iter()
                    .any(|(_, name)| name.eq_ignore_ascii_case(&claim.entity))
            {
                entries.push((
                    format!("claim-entity:{}", claim.fingerprint),
                    claim.entity.clone(),
                ));
            }
        }
        entries
    }

    fn open_intel_topology(&mut self, index: usize, actor: bool) {
        use argos_osint_core::brain_topology::TopologySeed;
        let Some(article) = self.intel_articles.get(self.intel_sel).cloned() else {
            return;
        };
        let revision = argos_osint_core::atlas_metadata::input_hash(&article);
        let seed = if actor {
            let Some((actor_ref, name)) = self.intel_topology_actor_entries().get(index).cloned()
            else {
                return;
            };
            TopologySeed::RetainedActor {
                run_id: article.run_id,
                article_id: article.id,
                revision,
                actor_ref,
                name,
            }
        } else {
            let Some((local_id, text)) = self.intel_topology_claim_entries().get(index).cloned()
            else {
                return;
            };
            let text = self
                .intel_brief_outputs
                .get("intel_brief_claims")
                .and_then(|out| out["claims"].as_array())
                .and_then(|claims| {
                    claims
                        .iter()
                        .find(|claim| claim["id"].as_str() == Some(&local_id))
                })
                .and_then(|claim| claim["text"].as_str())
                .map(str::to_owned)
                .or_else(|| {
                    self.intel_claims
                        .iter()
                        .find(|claim| claim.fingerprint == local_id)
                        .map(|claim| claim.claim.clone())
                })
                .unwrap_or(text);
            TopologySeed::RetainedClaim {
                run_id: article.run_id,
                article_id: article.id,
                revision,
                local_id,
                text,
                job_id: self.intel_brief_job.clone().unwrap_or_default(),
            }
        };
        self.open_topology(seed);
    }

    fn recenter_topology(&mut self) {
        if let Some(id) = self.topology_state.selected_node_id.clone() {
            self.recenter_topology_node(&id);
        }
    }

    fn recenter_topology_node(&mut self, node_id: &str) {
        use argos_osint_core::brain_topology::{TopologyNodeKind as K, TopologySeed as S};
        let Some(node) = self
            .topology_state
            .snapshot
            .as_ref()
            .and_then(|snap| snap.nodes.iter().find(|node| node.id == node_id))
            .cloned()
        else {
            return;
        };
        if let Some(seed) = node.topology_seed.clone() {
            self.open_topology(seed);
            return;
        }
        let seed = match node.kind {
            K::Claim => S::Claim(node.id),
            K::Entity => S::Actor(node.member_ids.first().cloned().unwrap_or(node.id)),
            K::ActorGroup => S::ActorGroup {
                name_key: node.id.trim_start_matches("actor-group:").to_string(),
                member_ids: node.member_ids,
            },
            K::Inference => S::Inference(node.id),
            K::Article => S::Article {
                run_id: node.run_id,
                article_id: node.article_id,
                revision: node.revision,
                originating_memory: None,
            },
            K::Topic | K::Source => {
                self.status = "This projection retains its source context; select a connected record to recenter".into();
                return;
            }
        };
        self.open_topology(seed);
    }

    fn reveal_topology_related(&mut self, index: usize) {
        let count = super::topology::related_edge_indices(&self.topology_state).len();
        let index = index.min(count.saturating_sub(1));
        self.topology_state.related_sel = index;
        // Rebuild from current state before testing visibility. Logical records
        // beyond this page enter focus through the same measured list renderer.
        let _ = super::ui::focus_order(self);
        let visible = self
            .layout
            .borrow()
            .entries
            .iter()
            .any(|entry| entry.target == Target::TopologyRelated(index));
        if index < self.topology_state.related_scroll || !visible {
            self.topology_state.related_scroll = index;
            self.layout.borrow_mut().view_stamp.clear();
        }
    }

    fn move_topology_row(&mut self, direction: isize) {
        match self.focus {
            Target::TopologyInspector => {
                super::topology::scroll_topology(self, Target::TopologyInspector, direction);
            }
            Target::TopologyRelated(_) | Target::TopologyRelationships => {
                let count = super::topology::related_edge_indices(&self.topology_state).len();
                if count == 0 {
                    return;
                }
                let index = match self.focus {
                    Target::TopologyRelated(index) => {
                        index.saturating_add_signed(direction).min(count - 1)
                    }
                    _ => 0,
                };
                self.set_focus(Target::TopologyRelated(index));
            }
            Target::TopologyGraph | Target::TopologyNode(_) | Target::TopologyEdge(_) => {
                let Some(snapshot) = self.topology_state.snapshot.as_ref() else {
                    return;
                };
                let eligible: Vec<_> = snapshot
                    .nodes
                    .iter()
                    .enumerate()
                    .filter(|(_, node)| super::topology::node_visible(&self.topology_state, node))
                    .map(|(index, _)| index)
                    .collect();
                if eligible.is_empty() {
                    return;
                }
                let current = eligible
                    .iter()
                    .position(|index| {
                        Some(&snapshot.nodes[*index].id)
                            == self.topology_state.selected_node_id.as_ref()
                    })
                    .unwrap_or(0);
                let index = eligible[current
                    .saturating_add_signed(direction)
                    .min(eligible.len() - 1)];
                self.activate_target(Target::TopologyNode(index));
                let _ = super::ui::focus_order(self);
                let visible = self
                    .layout
                    .borrow()
                    .entries
                    .iter()
                    .any(|entry| entry.target == Target::TopologyNode(index));
                if !visible {
                    self.topology_state.center_selected();
                    self.layout.borrow_mut().view_stamp.clear();
                }
            }
            _ => {}
        }
    }

    fn handle_topology_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Esc | KeyCode::Backspace => {
                self.activate_button(ButtonId::TopologyBack);
                true
            }
            KeyCode::Char('+') | KeyCode::Char('=') => {
                self.topology_state.zoom_percent = (self.topology_state.zoom_percent + 10).min(200);
                true
            }
            KeyCode::Char('-') | KeyCode::Char('_') => {
                self.topology_state.zoom_percent =
                    self.topology_state.zoom_percent.saturating_sub(10).max(50);
                true
            }
            KeyCode::Home => {
                self.topology_state.pan = (0, 0);
                self.topology_state.zoom_percent = 100;
                self.topology_state.viewport_center_id = None;
                true
            }
            KeyCode::Char('r') | KeyCode::Char('R') | KeyCode::F(5) => {
                self.refresh_topology();
                true
            }
            KeyCode::Char('c') => {
                self.recenter_topology();
                true
            }
            KeyCode::PageUp | KeyCode::PageDown => {
                super::ui::page(self, if key.code == KeyCode::PageUp { -1 } else { 1 });
                true
            }
            KeyCode::Tab => {
                self.focus_next(false);
                true
            }
            KeyCode::BackTab => {
                self.focus_next(true);
                true
            }
            KeyCode::Up => {
                if key.modifiers.contains(KeyModifiers::SHIFT) {
                    self.topology_state.pan.1 = self.topology_state.pan.1.saturating_sub(2);
                } else {
                    self.move_topology_row(-1);
                }
                true
            }
            KeyCode::Down => {
                if key.modifiers.contains(KeyModifiers::SHIFT) {
                    self.topology_state.pan.1 = self.topology_state.pan.1.saturating_add(2);
                } else {
                    self.move_topology_row(1);
                }
                true
            }
            KeyCode::Left => {
                if key.modifiers.contains(KeyModifiers::SHIFT) {
                    self.topology_state.pan.0 = self.topology_state.pan.0.saturating_sub(4);
                } else if let Target::TopologyPanel(index) = self.focus {
                    self.set_focus(Target::TopologyPanel(index.saturating_sub(1)));
                } else if let Target::TopologyInspectorTab(index) = self.focus {
                    self.set_focus(Target::TopologyInspectorTab(index.saturating_sub(1)));
                } else {
                    return false;
                }
                true
            }
            KeyCode::Right => {
                if key.modifiers.contains(KeyModifiers::SHIFT) {
                    self.topology_state.pan.0 = self.topology_state.pan.0.saturating_add(4);
                } else if let Target::TopologyPanel(index) = self.focus {
                    self.set_focus(Target::TopologyPanel((index + 1).min(2)));
                } else if let Target::TopologyInspectorTab(index) = self.focus {
                    self.set_focus(Target::TopologyInspectorTab((index + 1).min(1)));
                } else {
                    return false;
                }
                true
            }
            KeyCode::Enter | KeyCode::Char(' ') => {
                self.activate_target(self.focus);
                true
            }
            _ => false,
        }
    }

    /// One transition for every memory link (Brain list, Related, others).
    /// Fetches the destination by id, even when Find hides it. On failure the
    /// current view stays intact. Returns whether the detail opened.
    pub(crate) fn open_memory_detail(&mut self, memory_id: &str) -> bool {
        if self.background_db.is_some() {
            self.request_brain_detail(memory_id);
            return true;
        }
        if memory_id.starts_with("actor-group:") {
            return self.open_actor_group(memory_id);
        }
        if !self.store.is_browse_claim(memory_id).unwrap_or(false) {
            self.status = "That record is unavailable in Brain".into();
            return false;
        }
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

    pub(crate) fn open_actor_group(&mut self, key: &str) -> bool {
        if self.background_db.is_some() {
            self.request_brain_detail(key);
            return true;
        }
        let group = match self.store.actor_group(key) {
            Ok(Some(group)) => group,
            _ => {
                self.status = "That actor group is unavailable".into();
                return false;
            }
        };
        let graph = match self.store.actor_group_graph(&group) {
            Ok(graph) => graph,
            Err(e) => {
                self.status = format!("Could not load actor associations: {e}");
                return false;
            }
        };
        if let Some(snapshot) = self.detail_snapshot() {
            if snapshot.memory_id != group.key() {
                self.brain_detail.push(snapshot);
            }
        }
        let memory = Memory {
            id: group.key(),
            text: group.display_name,
            category: "actor group".into(),
            pinned: false,
            created_at: String::new(),
            source: MemorySource {
                app: "brain-actor-group".into(),
                conversation_id: group.name_key,
                message_id: None,
                reference: serde_json::to_string(&group.actor_ids).ok(),
            },
        };
        self.show_detail(memory, graph, None);
        true
    }

    fn request_brain_detail(&mut self, id: &str) {
        if self.brain_detail_reading && self.brain_detail_target.as_deref() == Some(id) {
            return;
        }
        self.brain_detail_restore = None;
        self.brain_detail_back = false;
        self.brain_detail_target = Some(id.to_string());
        self.brain_detail_generation = self.brain_detail_generation.wrapping_add(1);
        self.start_brain_detail_read();
        self.status = "Loading Brain detail…".into();
    }

    pub(crate) fn brain_refreshing(&self) -> bool {
        self.brain_reading.is_some()
    }

    pub(crate) fn intel_bulletin_loading(&self) -> bool {
        self.intel_list_reading
    }
    pub(crate) fn atlas_news_loading(&self) -> bool {
        self.atlas_articles_reading
    }

    fn start_brain_detail_read(&mut self) {
        if self.brain_detail_reading || self.module != Some(ModuleId::Brain) {
            return;
        }
        let (Some(db), Some(id)) = (self.background_db.clone(), self.brain_detail_target.clone())
        else {
            return;
        };
        self.brain_detail_reading = true;
        let generation = self.brain_detail_generation;
        let tx = self.work_tx.clone();
        std::thread::spawn(move || {
            let outcome = Store::open_profile_reader(&db)
                .and_then(|store| {
                    store.connection().execute_batch("BEGIN")?;
                    load_brain_detail(&store, &id).map(Box::new)
                })
                .map_err(|e| e.to_string());
            let _ = tx.send(WorkEvent::BrainDetailLoaded {
                generation,
                outcome,
            });
        });
    }

    fn on_brain_detail_loaded(
        &mut self,
        generation: u64,
        outcome: Result<Box<LoadedBrainDetail>, String>,
    ) {
        self.brain_detail_reading = false;
        if generation != self.brain_detail_generation {
            self.start_brain_detail_read();
            return;
        }
        if self.module != Some(ModuleId::Brain) || self.brain_detail_target.take().is_none() {
            return;
        }
        match outcome {
            Ok(detail) => {
                let restore = self.brain_detail_restore.take();
                if self.brain_detail_back {
                    self.brain_detail.history.pop();
                } else if restore.is_none() {
                    if let Some(snapshot) = self.detail_snapshot() {
                        if snapshot.memory_id != detail.memory.id {
                            self.brain_detail.push(snapshot);
                        }
                    } else {
                        self.brain_detail.history.clear();
                    }
                }
                self.brain_associations = Some(detail.associations);
                self.brain_typed = detail.typed;
                self.selected_insight = detail.insight;
                self.brain_detail_claim = detail.claim;
                self.show_detail(detail.memory, detail.graph, restore.as_ref());
                self.status = "Brain detail".into();
            }
            Err(error) => {
                if error == "claim unavailable" || error == "actor unavailable" {
                    if self.brain_detail_back {
                        self.brain_detail.history.pop();
                        self.detail_back();
                        return;
                    }
                    if self.brain_detail_restore.is_some() {
                        self.leave_brain_detail();
                    }
                }
                self.status = format!("Could not load Brain detail: {error}");
                self.push_log_detail("error", "Brain: could not load detail", error);
            }
        }
    }

    fn show_detail(
        &mut self,
        memory: Memory,
        graph: recon::MemoryGraph,
        restore: Option<&super::brain_detail::DetailSnapshot>,
    ) {
        self.brain_list_mode = BrainListMode::Graph;
        self.brain_graph = graph;
        if self.background_db.is_none() {
            self.brain_typed = argos_osint_core::brain_kinds::typed_detail(&self.store, &memory.id)
                .unwrap_or_default();
            self.selected_insight = self.store.insight_for_memory(&memory.id).ok().flatten();
        }
        self.brain_graph_for = Some(memory.id.clone());
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
        let actor = memory.id.starts_with("actor-group:");
        self.compact_pages[ModuleId::Brain.index()] = restore.map_or(0, |s| s.compact_page);
        self.brain_detail.related.tab = restore.map_or(
            if actor {
                RelatedTab::Claims
            } else {
                RelatedTab::Actors
            },
            |s| s.tab,
        );
        self.brain_detail.related.cursors = restore.map_or([(0, 0); 3], |s| s.cursors);
        self.brain_detail.related.page_sizes = restore.map_or([20; 3], |s| s.page_sizes);
        self.brain_detail.related.selected_keys =
            restore.map_or([None, None, None], |s| s.selected_keys.clone());
        self.request_related(request, &memory.id);
        if actor {
            let a = self.brain_associations.clone().unwrap_or_else(|| {
                self.store
                    .actor_group(&memory.id)
                    .ok()
                    .flatten()
                    .and_then(|g| self.store.actor_group_associations(&g).ok())
                    .unwrap_or_default()
            });
            self.graph_summary = format!(
                "{} claims · {} threads · {} articles",
                a.claims.len(),
                a.threads.len(),
                a.articles.len()
            );
            self.brain_typed = vec![self.graph_summary.clone()];
            self.graph_summary_pending = None;
        } else {
            self.load_or_request_summary(&memory);
        }
        let focus = restore.map_or(Target::DetailPath, |s| s.focus);
        self.set_focus(match focus {
            Target::RelatedRow(_) => Target::RelatedRow(self.brain_detail.related.sel),
            other => other,
        });
    }

    /// Related items for `memory_id`: on a blocking worker in the live app, inline
    /// otherwise. Late results for another request or memory are dropped.
    fn request_related(&mut self, request: u64, memory_id: &str) {
        let tab = self.brain_detail.related.tab;
        if self.background_db.is_some() {
            if let Some(a) = &self.brain_associations {
                self.on_related(
                    request,
                    memory_id,
                    Ok(association_rows_from(a.clone(), tab)),
                );
                return;
            }
        }
        if let (Ok(_), Some(db)) = (tokio::runtime::Handle::try_current(), tracked::db_path()) {
            let tx = self.work_tx.clone();
            let memory_id = memory_id.to_string();
            tokio::task::spawn_blocking(move || {
                let outcome = Store::open_profile_reader(&db)
                    .and_then(|store| association_rows(&store, &memory_id, tab))
                    .map_err(|e| e.to_string());
                let _ = tx.send(WorkEvent::Related {
                    request,
                    memory_id,
                    outcome,
                });
            });
            return;
        }
        let outcome = association_rows(&self.store, memory_id, tab).map_err(|e| e.to_string());
        self.on_related(request, memory_id, outcome);
    }

    fn on_related(
        &mut self,
        request: u64,
        memory_id: &str,
        outcome: std::result::Result<Vec<AssociationItem>, String>,
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
            self.focus = Target::RelatedRow(self.brain_detail.related.sel);
        }
        true
    }

    fn select_brain_related_tab(&mut self, index: usize) {
        let Some(id) = self.brain_detail.memory_id().map(str::to_string) else {
            return;
        };
        let actor = id.starts_with("actor-group:");
        let tabs: &[RelatedTab] = if actor {
            &[
                RelatedTab::Claims,
                RelatedTab::Threads,
                RelatedTab::Articles,
            ]
        } else {
            &[RelatedTab::Actors, RelatedTab::Sources]
        };
        let Some(tab) = tabs.get(index).copied() else {
            return;
        };
        let view = &mut self.brain_detail.related;
        view.cursors[view.tab.index()] = (view.sel, view.scroll);
        view.selected_keys[view.tab.index()] =
            view.items.get(view.sel).map(|item| item.key.clone());
        view.tab = tab;
        let (sel, scroll) = view.cursors[index];
        let request = self.brain_detail.begin_related(sel, scroll);
        self.request_related(request, &id);
    }

    /// Back: the previous detail memory with its selection, focus, and scroll;
    /// at the first detail, the Brain list with its Find and selection.
    fn detail_back(&mut self) {
        if self.background_db.is_some() {
            if let Some(snapshot) = self.brain_detail.history.last().cloned() {
                self.request_brain_detail(&snapshot.memory_id);
                self.brain_detail_restore = Some(snapshot);
                self.brain_detail_back = true;
            } else {
                self.leave_brain_detail();
            }
            return;
        }
        while let Some(snapshot) = self.brain_detail.history.pop() {
            if snapshot.memory_id.starts_with("actor-group:") {
                let depth = self.brain_detail.history.len();
                if self.open_actor_group(&snapshot.memory_id) {
                    self.brain_detail.history.truncate(depth);
                    if let Some(memory) = self.brain_detail.memory.clone() {
                        self.show_detail(memory, self.brain_graph.clone(), Some(&snapshot));
                    }
                    return;
                }
                continue;
            }
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
        let len = self.brain_detail.related.items.len().min(
            self.brain_detail.related.page_sizes[self.brain_detail.related.tab.index()].max(20),
        );
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
        let Some(item) = self.brain_detail.selected_related().cloned() else {
            return;
        };
        match item.target {
            AssociationTarget::Claim(id) => {
                self.open_memory_detail(&id);
            }
            AssociationTarget::ActorGroup(key) => {
                self.open_actor_group(&key);
            }
            AssociationTarget::Thread(id) => {
                if self.store.get_thread(&id).ok().flatten().is_none() {
                    self.status = "That thread is unavailable".into();
                    return;
                }
                self.brain_destination_back = self.detail_snapshot();
                if let Err(e) = self.enter_investigation(&id) {
                    self.brain_destination_back = None;
                    self.status = e.to_string();
                }
            }
            AssociationTarget::Article {
                run_id,
                article_id,
                revision,
            } => {
                let Some(article) = self
                    .store
                    .atlas_article(&run_id, &article_id)
                    .ok()
                    .flatten()
                    .filter(|a| argos_osint_core::atlas_metadata::input_hash(a) == revision)
                else {
                    self.status =
                        "That source article is unavailable; select the tab to refresh".into();
                    return;
                };
                self.brain_destination_back = self.detail_snapshot();
                if let Err(e) = self.open_intel_briefing_for(article) {
                    self.brain_destination_back = None;
                    self.status = e.to_string();
                }
            }
        }
    }

    fn open_related_article(&mut self, index: usize) {
        let view = &self.intel_related;
        let Some(item) = view.page.rows.get(index) else {
            return;
        };
        let article = match self
            .store
            .atlas_article(&item.article.run_id, &item.article.id)
        {
            Ok(Some(article))
                if argos_osint_core::atlas_metadata::input_hash(&article)
                    == argos_osint_core::atlas_metadata::input_hash(&item.article)
                    && article.category == item.article.category =>
            {
                article
            }
            _ => {
                self.status = "That related article is unavailable; refresh the list".into();
                return;
            }
        };
        let back = {
            let Some(current) = self.intel_articles.get(self.intel_sel).cloned() else {
                return;
            };
            RelatedArticleBack::Article {
                article: Box::new(current),
                related: Box::new(super::related_articles::ArticleRelatedView {
                    sel: index,
                    ..view.clone()
                }),
                extracted_scroll: self.scrolls.intel_extracted,
                brief_scroll: self.scrolls.intel_brief,
                full_scroll: self.scrolls.intel_full,
                focus: Target::IntelRelatedArticle(index),
            }
        };
        self.intel_related.sel = index;
        if self.related_back.len() == super::brain_detail::HISTORY_CAP {
            self.related_back.remove(0);
        }
        self.related_back.push(back);
        match self.open_intel_briefing_for(article) {
            Ok(status) => self.status = status,
            Err(error) => {
                self.related_back.pop();
                self.status = error.to_string();
            }
        }
    }

    fn more_related_articles(&mut self) -> Result<String> {
        if self.background_db.is_some() {
            self.intel_related_limit = self
                .intel_related
                .page
                .rows
                .len()
                .max(self.intel_related_limit)
                .saturating_add(20);
            self.refresh_intel_briefing();
            return Ok("Loading more related articles…".into());
        }
        let offset = self.intel_related.page.rows.len();
        let article = self
            .intel_articles
            .get(self.intel_sel)
            .ok_or_else(|| anyhow::anyhow!("No article selected"))?;
        let page = self
            .store
            .related_articles_global(
                &article.run_id,
                &article.id,
                self.related_scope,
                Some(&offset.to_string()),
                20,
            )?
            .into_page(offset);
        self.intel_related.page.rows.extend(page.rows);
        self.intel_related.page.total = page.total;
        self.intel_related.page.has_more = page.has_more;
        Ok("Related articles loaded".into())
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
                self.compact_pages[ModuleId::Providers.index()] = 1;
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
            ButtonId::BrainKind => {
                self.cycle_brain_kind();
                return;
            }
            ButtonId::RelatedScope => {
                self.toggle_related_scope();
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
            ButtonId::BrainRelatedFirst => {
                self.select_brain_related_tab(0);
                return;
            }
            ButtonId::BrainRelatedSecond => {
                self.select_brain_related_tab(1);
                return;
            }
            ButtonId::BrainAssociationsMore => {
                let view = &mut self.brain_detail.related;
                view.page_sizes[view.tab.index()] =
                    view.page_sizes[view.tab.index()].max(20).saturating_add(20);
                return;
            }
            ButtonId::BrainRelatedThird => {
                self.select_brain_related_tab(2);
                return;
            }
            ButtonId::IntelRelatedMore => self.more_related_articles(),
            ButtonId::BrainDetailBack => {
                self.detail_back();
                return;
            }
            ButtonId::BrainMapTopology => {
                if let Some(id) = self.brain_detail.memory_id() {
                    let seed = if id.starts_with("actor-group:") {
                        let name_key = id.trim_start_matches("actor-group:").to_string();
                        argos_osint_core::brain_topology::TopologySeed::ActorGroup {
                            name_key,
                            member_ids: self
                                .brain_detail
                                .memory
                                .as_ref()
                                .and_then(|m| m.source.reference.as_deref())
                                .and_then(|s| serde_json::from_str(s).ok())
                                .unwrap_or_default(),
                        }
                    } else if id.starts_with("actor:") {
                        argos_osint_core::brain_topology::TopologySeed::Actor(id.to_string())
                    } else {
                        argos_osint_core::brain_topology::TopologySeed::Claim(id.to_string())
                    };
                    self.open_topology(seed);
                }
                return;
            }
            ButtonId::TopologyBack => {
                if let Some(frame) = self.topology_state.pop_history() {
                    self.topology_state.restore_history(frame);
                    self.topology_state.snapshot = None;
                    self.refresh_topology();
                } else if let Some(origin) = self.topology_origin.take() {
                    self.topology_generation = self.topology_generation.wrapping_add(1);
                    self.topology_loading = false;
                    self.module = origin.module;
                    self.brain_list_mode = origin.brain_mode;
                    self.intel_page = origin.intel_page;
                    self.compact_pages = origin.compact_pages;
                    self.scrolls = origin.scrolls;
                    if let Some(id) = origin.article_id {
                        if let Some(index) = self.intel_articles.iter().position(|a| a.id == id) {
                            self.intel_sel = index;
                        }
                    }
                    let unchanged =
                        origin
                            .article_revision
                            .as_ref()
                            .is_none_or(|(run, revision)| {
                                self.intel_articles
                                    .get(self.intel_sel)
                                    .is_some_and(|article| {
                                        &article.run_id == run
                                            && argos_osint_core::atlas_metadata::input_hash(article)
                                                == *revision
                                    })
                            });
                    self.focus = origin.focus;
                    self.status = if unchanged {
                        "Returned to topology entry".into()
                    } else {
                        "Origin article changed or is unavailable · showing retained owner context"
                            .into()
                    };
                    if !unchanged && self.module == Some(ModuleId::Intel) {
                        self.intel_page = IntelPage::Bulletin;
                        self.set_focus(Target::IntelArticle(self.intel_sel));
                    } else if self.module == Some(ModuleId::Brain)
                        && self.brain_list_mode == BrainListMode::Graph
                    {
                        if self.brain_detail.memory.is_some() {
                            self.refresh_open_detail();
                        } else {
                            self.leave_brain_detail();
                            self.status = "Origin memory is unavailable · returned to Brain".into();
                        }
                    }
                } else if self.brain_detail.memory.is_some() {
                    self.brain_list_mode = BrainListMode::Graph;
                    self.set_focus(Target::Button(ButtonId::BrainDetailBack));
                } else {
                    self.brain_list_mode = BrainListMode::List;
                    self.set_focus(Target::Memory(self.memory_sel));
                }
                return;
            }
            ButtonId::TopologyRecenter => {
                self.recenter_topology();
                return;
            }
            ButtonId::TopologyZoomIn => {
                self.topology_state.zoom_percent = (self.topology_state.zoom_percent + 10).min(200);
                return;
            }
            ButtonId::TopologyZoomOut => {
                self.topology_state.zoom_percent =
                    self.topology_state.zoom_percent.saturating_sub(10).max(50);
                return;
            }
            ButtonId::TopologyOpenMemory => {
                if let Some(node) = self.topology_state.snapshot.as_ref().and_then(|snap| {
                    snap.nodes.iter().find(|node| {
                        Some(&node.id) == self.topology_state.selected_node_id.as_ref()
                    })
                }) {
                    self.overlay = Overlay::Block {
                        title: format!("{} · {}", node.kind.label(), node.label),
                        body: format!(
                            "{}\n\nRecord: {}\nStatus: {}\nRevision: {}\n\n{}",
                            node.detail,
                            node.id,
                            node.status,
                            node.revision,
                            node.passages.join("\n\n")
                        ),
                    };
                    self.scrolls.popup = 0;
                }
                return;
            }
            ButtonId::TopologyViewEvidence => {
                let next = usize::from(self.topology_state.inspector_tab == 0);
                self.activate_target(Target::TopologyInspectorTab(next));
                return;
            }
            ButtonId::TopologyScopeAll => {
                self.topology_state.cycle_scope =
                    argos_osint_core::brain_topology::CycleScope::AllRetained;
                self.topology_state.snapshot = None;
                self.refresh_topology();
                return;
            }
            ButtonId::TopologyScopeThis => {
                if let Some(cycle_id) = self.topology_state.current_cycle_id() {
                    self.topology_state.cycle_scope =
                        argos_osint_core::brain_topology::CycleScope::ThisCycle(cycle_id);
                    self.topology_state.snapshot = None;
                    self.refresh_topology();
                }
                return;
            }
            ButtonId::TopologyDepth1 => {
                self.topology_state.depth = 1;
                self.topology_state.snapshot = None;
                self.refresh_topology();
                return;
            }
            ButtonId::TopologyDepth2 => {
                self.topology_state.depth = 2;
                self.topology_state.snapshot = None;
                self.refresh_topology();
                return;
            }
            ButtonId::TopologyDepth3 => {
                self.topology_state.depth = 3;
                self.topology_state.snapshot = None;
                self.refresh_topology();
                return;
            }
            ButtonId::TopologyCenterSelected => {
                self.topology_state.center_selected();
                return;
            }
            ButtonId::TopologyFitGraph => {
                self.topology_state.pan = (0, 0);
                self.topology_state.zoom_percent = 100;
                self.topology_state.viewport_center_id = None;
                return;
            }
            ButtonId::TopologyAnalyzeScope => {
                if let Some(seed) = self.topology_state.seed.clone() {
                    let options = argos_osint_core::brain_topology::TopologyOptions {
                        depth: self.topology_state.depth,
                        cycle_scope: self.topology_state.cycle_scope.clone(),
                        max_nodes: 100,
                        max_edges: 200,
                        continuation_token: None,
                    };
                    let payload = argos_osint_core::brain_scope::ScopeAnalysisPayload {
                        seed,
                        cycle_scope: self.topology_state.cycle_scope.clone(),
                        depth: self.topology_state.depth,
                        options,
                    };
                    let now = chrono::Utc::now().to_rfc3339();
                    match argos_osint_core::brain_scope::enqueue_scope_analysis(
                        self.store.connection(),
                        &payload,
                        &now,
                    ) {
                        Ok(task_id) => {
                            self.topology_state.analyzing_scope = true;
                            self.topology_state.scope_status = format!("Enqueued ({task_id})");
                            self.status = format!("Enqueued scope analysis ({task_id})");
                        }
                        Err(e) => {
                            self.topology_state.scope_status = format!("Failed: {e}");
                            self.status = format!("Failed to analyze scope: {e}");
                        }
                    }
                }
                return;
            }
            ButtonId::Add => self.save_insight(),
            ButtonId::Pin => {
                let Some(memory) = (if self.brain_list_mode == BrainListMode::Graph {
                    self.brain_detail.memory.as_ref()
                } else {
                    self.memories.get(self.memory_sel)
                }) else {
                    self.status = "No memory selected".into();
                    return;
                };
                if memory.id.starts_with("actor-group:") {
                    self.status = "Actor groups are read-only".into();
                    return;
                }
                self.store
                    .update_memory(&memory.id, &memory.text, &memory.category, !memory.pinned)
                    .map(|_| {
                        self.reload_memories();
                        "Memory pin updated".into()
                    })
            }
            ButtonId::Delete => {
                let Some(memory) = (if self.brain_list_mode == BrainListMode::Graph {
                    self.brain_detail.memory.as_ref()
                } else {
                    self.memories.get(self.memory_sel)
                }) else {
                    self.status = "No memory selected".into();
                    return;
                };
                if memory.id.starts_with("actor-group:") {
                    self.status = "Actor groups are read-only".into();
                    return;
                }
                if self.background_db.is_some() {
                    self.request_memory_delete(memory.id.clone())
                } else {
                    self.store.delete_memory(&memory.id).map(|_| {
                        self.reload_memories();
                        "Memory deleted".into()
                    })
                }
            }
            ButtonId::NewThread => self.new_thread().map(|_| "New investigation".into()),
            ButtonId::DeleteThread => {
                if self.background_db.is_some() {
                    let result = self.request_thread_delete();
                    self.report(result);
                    return;
                }
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
                    self.investigation_events.clear();
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
            ButtonId::ClearLog => {
                // Scope: durable events only. Jobs, task results and memories stay.
                if let Some(db) = self.background_db.clone() {
                    if self.logs_clearing {
                        self.status = "Events cleanup is already in progress".into();
                        return;
                    }
                    self.logs_clearing = true;
                    let tx = self.work_tx.clone();
                    std::thread::spawn(move || {
                        let outcome = Store::open_light(&db)
                            .and_then(|store| store.clear_events())
                            .map_err(|error| error.to_string());
                        let _ = tx.send(WorkEvent::LogsCleared { outcome });
                    });
                    self.status = "Clearing events…".into();
                    return;
                }
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
                        "Stop requested · the job stops at its next safe point".into()
                    }
                    CancelRequest::NotCancellable => "This job cannot be cancelled safely".into(),
                    CancelRequest::NotRunning => "This job is not running".into(),
                    CancelRequest::Missing => "Job history expired".into(),
                })
            }
            ButtonId::JobsOpenSource => self.open_job_source(),
            ButtonId::JobsClearCompleted
            | ButtonId::JobsClearStopped
            | ButtonId::JobsClearFailed => {
                use argos_osint_core::jobs_view::JobStatusFilter;
                let status = match button {
                    ButtonId::JobsClearCompleted => JobStatusFilter::Completed,
                    ButtonId::JobsClearStopped => JobStatusFilter::Stopped,
                    _ => JobStatusFilter::Failed,
                };
                if let Some(db) = self.background_db.clone() {
                    if self.jobs_clearing {
                        self.status = "Jobs cleanup is already in progress".into();
                        return;
                    }
                    self.jobs_clearing = true;
                    let tx = self.work_tx.clone();
                    std::thread::spawn(move || {
                        let outcome = Store::open_light(&db)
                            .and_then(|store| store.clear_jobs_by_status(status))
                            .map(|counts| {
                                format!(
                                    "Cleared {} jobs · stopped {}",
                                    counts.cleared, counts.stopped
                                )
                            })
                            .map_err(|e| e.to_string());
                        let _ = tx.send(WorkEvent::JobsCleared { outcome });
                    });
                    self.status =
                        format!("Clearing {} job history…", status.label().to_lowercase());
                    return;
                }
                let result = self.store.clear_jobs_by_status(status).map(|counts| {
                    format!(
                        "Cleared {} jobs · stopped {}",
                        counts.cleared, counts.stopped
                    )
                });
                self.jobs.scroll = 0;
                self.jobs.detail_scroll = 0;
                self.reload_jobs();
                self.refresh_intel_briefing();
                self.sync_intel_brief_task_ui();
                self.set_focus(Target::Button(button));
                result
            }
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
                self.compact_pages[ModuleId::Providers.index()] = 1;
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
            ButtonId::BriefFastPreset => self.toggle_fast_preset(),
            ButtonId::BriefOpCycle => Ok(self.cycle_brief_op()),
            ButtonId::BriefOpSave => self.save_brief_op_override(),
            ButtonId::BriefOpReset => self.reset_brief_op_override(),
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
            ButtonId::IntelExtractionRetry => self.retry_selected_extraction(),
            ButtonId::IntelExtractionJob => self.open_selected_extraction_job(),
            ButtonId::IntelBodyMore => {
                if let super::ui::IntelBodyPresentation::Content { markdown, .. } =
                    super::ui::intel_body_presentation(self)
                {
                    let body = markdown.to_owned();
                    self.overlay = Overlay::Block {
                        title: "Full article".into(),
                        body,
                    };
                    self.scrolls.popup = 0;
                    self.set_focus(Target::CloseOverlay);
                }
                Ok("Full article".into())
            }
            ButtonId::JumpLatest => {
                self.chat_follow = true;
                super::ui::normalize(self);
                return;
            }
            ButtonId::IntelVisit => {
                if let Some(article) = self.intel_articles.get(self.intel_sel) {
                    if let Err(error) = open_external_url(&article.url) {
                        self.status = format!("Could not open {}: {error}", article.url);
                    }
                }
                return;
            }
            ButtonId::IntelBodyRetry | ButtonId::IntelBodyRefresh => {
                if let Some(job) = self.intel_brief_job.clone() {
                    if let Ok(Some(_)) = intel_recon::brief_pipeline::retry_stage(
                        self.store.connection(),
                        &job,
                        &chrono::Utc::now().to_rfc3339(),
                    ) {
                        self.refresh_intel_brief_stages();
                        self.status = "Retrying failed brief stage; completed stages kept".into();
                        return;
                    }
                }
                if let Some(article) = self.intel_articles.get(self.intel_sel).cloned() {
                    match intel_recon::brief_stages::reload_intel_brief_pipeline(
                        &self.store,
                        &article,
                        &self.settings,
                    ) {
                        Ok(job) => {
                            self.intel_brief_job = Some(job);
                            self.intel_body = None;
                            self.intel_brief_outputs.clear();
                            self.intel_brief_stages.clear();
                            self.intel_body_owner = None;
                            self.intel_body_require_commit = true;
                            self.intel_body_read_error = None;
                            self.layout.borrow_mut().view_stamp.clear();
                            self.refresh_intel_brief_stages();
                            self.status = "Reloading full article through brief preparation".into();
                        }
                        Err(error) => self.status = error.to_string(),
                    }
                }
                return;
            }
            ButtonId::IntelJobOpen => {
                let result = self.open_selected_extraction_job();
                self.report(result);
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

    pub(crate) fn activate_target(&mut self, target: Target) {
        match target {
            Target::ConfigAction(action) => {
                self.set_focus(target);
                self.config_action(action);
            }
            Target::PaneTab(index) => {
                if let Some(module) = self.module {
                    self.compact_pages[module.index()] = index;
                }
                self.set_focus(target);
            }
            Target::ProfileTab(_)
            | Target::ProfileApp(_)
            | Target::ProfileAction(_)
            | Target::ProfileCard(_)
            | Target::ProfileChoice(_)
            | Target::ProfileBucket(_)
            | Target::ProfilePanelBucket(_, _)
            | Target::ProfilePanelRow(_, _)
            | Target::ProfileReport
            | Target::ProfileTable(_)
            | Target::ProfileTableRow(_, _)
            | Target::ProfileRow(_)
            | Target::ProfileSystem(_) => {
                self.set_focus(target);
                super::profile::activate(self, target);
            }
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
            Target::ToolCategory(category) => {
                if !self.tool_expanded.remove(category) {
                    self.tool_expanded.insert(category);
                }
                self.set_focus(Target::ToolCategory(category));
            }
            Target::OsintResponse => self.set_focus(target),
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
            Target::IntelRelatedArticle(index) => self.open_related_article(index),
            Target::RelatedRow(index) => {
                // A click opens the row under the pointer immediately.
                if index < self.brain_detail.related.items.len() {
                    self.brain_detail.related.sel = index;
                    self.set_focus(Target::RelatedRow(index));
                    self.open_selected_related();
                }
            }
            Target::DetailPath => {
                self.set_focus(target);
                self.activate_button(ButtonId::BrainMapTopology);
            }
            Target::DetailSummary => self.set_focus(target),
            Target::IntelTopologyClaim(index) => {
                self.focus = target;
                self.open_intel_topology(index, false);
            }
            Target::IntelTopologyActor(index) => {
                self.focus = target;
                self.open_intel_topology(index, true);
            }
            Target::TopologyPanel(index) => {
                self.topology_state.active_panel_tab = index.min(2);
                let mode = super::topology_layout::TopologyLayoutResult::compute(
                    super::ui::body_rect(self),
                )
                .mode;
                let focus = if index == 1
                    && matches!(
                        mode,
                        super::topology_layout::TopologyResponsiveMode::Desktop
                            | super::topology_layout::TopologyResponsiveMode::Compact
                    ) {
                    Target::TopologyInspector
                } else {
                    target
                };
                self.layout.borrow_mut().view_stamp.clear();
                self.set_focus(focus);
            }
            Target::TopologyInspectorTab(index) => {
                self.topology_state.inspector_tab = index.min(1);
                self.topology_state.related_scroll = 0;
                self.topology_state.related_sel = 0;
                self.layout.borrow_mut().view_stamp.clear();
                self.set_focus(target);
            }
            Target::TopologyFilter(index) => {
                let filter = match index {
                    0 => &mut self.topology_state.filter_actor,
                    1 => &mut self.topology_state.filter_claim,
                    2 => &mut self.topology_state.filter_inference,
                    3 => &mut self.topology_state.filter_link,
                    4 => &mut self.topology_state.filter_related,
                    _ => &mut self.topology_state.filter_sources,
                };
                *filter = !*filter;
                if let Some(snapshot) = &self.topology_state.snapshot {
                    let hidden = snapshot
                        .nodes
                        .iter()
                        .find(|node| {
                            Some(&node.id) == self.topology_state.selected_node_id.as_ref()
                        })
                        .is_some_and(|node| {
                            !super::topology::node_visible(&self.topology_state, node)
                        });
                    if hidden {
                        self.topology_state.selected_node_id = Some(snapshot.root_id.clone());
                        self.topology_state.selected_edge_id = None;
                        self.topology_state.inspector_scroll = 0;
                    }
                }
                self.topology_state.related_sel = 0;
                self.topology_state.related_scroll = 0;
                self.layout.borrow_mut().view_stamp.clear();
                self.set_focus(target);
            }
            Target::TopologyEndpoint(index) => {
                let endpoint = self
                    .topology_state
                    .snapshot
                    .as_ref()
                    .and_then(|snap| {
                        snap.edges.iter().find(|edge| {
                            Some(&edge.id) == self.topology_state.selected_edge_id.as_ref()
                        })
                    })
                    .map(|edge| {
                        if index == 0 {
                            edge.from.clone()
                        } else {
                            edge.to.clone()
                        }
                    });
                if let Some(endpoint) = endpoint {
                    self.focus = target;
                    self.recenter_topology_node(&endpoint);
                }
            }
            Target::TopologyGraph => {
                self.set_focus(Target::TopologyGraph);
            }
            Target::TopologyInspector | Target::TopologyRelationships => self.set_focus(target),
            Target::TopologyNode(index) => {
                if let Some(snap) = &self.topology_state.snapshot {
                    if let Some(node) = snap.nodes.get(index) {
                        self.topology_state.selected_node_id = Some(node.id.clone());
                        self.topology_state.selected_edge_id = None;
                        self.topology_state.inspector_scroll = 0;
                        self.topology_state.related_sel = 0;
                        self.topology_state.related_scroll = 0;
                        self.set_focus(Target::TopologyNode(index));
                    }
                }
            }
            Target::TopologyEdge(index) => {
                if let Some(snap) = &self.topology_state.snapshot {
                    if let Some(edge) = snap.edges.get(index) {
                        self.topology_state.selected_edge_id = Some(edge.id.clone());
                        self.topology_state.inspector_scroll = 0;
                        self.set_focus(Target::TopologyEdge(index));
                    }
                }
            }
            Target::TopologyRelated(index) => {
                self.topology_state.related_sel = index;
                if let Some(edge_index) =
                    super::topology::related_edge_indices(&self.topology_state)
                        .get(index)
                        .copied()
                {
                    if let Some(edge) = self
                        .topology_state
                        .snapshot
                        .as_ref()
                        .and_then(|snap| snap.edges.get(edge_index))
                    {
                        self.topology_state.selected_edge_id = Some(edge.id.clone());
                        self.topology_state.inspector_scroll = 0;
                    }
                }
                self.set_focus(Target::TopologyRelated(index));
            }
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
                if self.background_db.is_some() {
                    self.pending_job = None;
                    self.jobs.select_cached(index);
                    self.reload_jobs();
                } else {
                    self.jobs.select(index, &self.store);
                }
                self.set_focus(Target::JobRow(self.jobs.sel));
            }
            Target::JobDetail => self.set_focus(Target::JobDetail),
            Target::IntelLeftColumn | Target::IntelPreview | Target::IntelFullArticle => {
                self.set_focus(target)
            }
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
            Target::CloseOverlay => {
                let restore = if self.overlay == Overlay::AddFallback {
                    self.fallback_popup_restore.take()
                } else if self.overlay == Overlay::Palette {
                    self.palette_restore.take()
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
        if self.focus == Target::Field(FieldId::ProfileImportEditor) {
            if self.profile.tab == super::profile::SystemTab::Configs
                || self.overlay == Overlay::Configs
            {
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
            self.filter_memory_cache();
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
            if self.background_db.is_some() {
                return self.request_thread_delete();
            }
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
            self.investigation_events.clear();
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

    fn request_thread_delete(&mut self) -> Result<String> {
        let id = self
            .selected_thread
            .clone()
            .ok_or_else(|| anyhow::anyhow!("No thread selected"))?;
        anyhow::ensure!(
            self.threads_deleting.insert(id.clone()),
            "This investigation is already being deleted"
        );
        self.running
            .remove(&id)
            .inspect(|cancel| cancel.store(true, Ordering::Relaxed));
        let db = self.background_db.clone().expect("production database");
        let tx = self.work_tx.clone();
        std::thread::spawn(move || {
            let outcome = Store::open_light(&db)
                .and_then(|mut store| {
                    let removed = store.deletion_consequences(&id)?.len();
                    store.delete_thread(&id, true)?;
                    Ok(removed)
                })
                .map_err(|e| e.to_string());
            let _ = tx.send(WorkEvent::ThreadDeleted {
                thread_id: id,
                outcome,
            });
        });
        Ok("Deleting investigation…".into())
    }

    fn request_memory_delete(&mut self, id: String) -> Result<String> {
        anyhow::ensure!(
            self.memories_deleting.insert(id.clone()),
            "This memory is already being deleted"
        );
        let db = self.background_db.clone().expect("production database");
        let tx = self.work_tx.clone();
        std::thread::spawn(move || {
            let outcome = Store::open_light(&db)
                .and_then(|store| store.delete_memory(&id))
                .map_err(|e| e.to_string());
            let _ = tx.send(WorkEvent::MemoryDeleted {
                memory_id: id,
                outcome,
            });
        });
        Ok("Deleting memory…".into())
    }

    fn finish_thread_delete(&mut self, id: &str) {
        self.tab_ids.retain(|tid| tid != id);
        self.tab_recently_closed.retain(|tid| tid != id);
        self.tab_last_active.remove(id);
        self.tab_unreads.remove(id);
        self.save_session_tabs();
        self.threads.retain(|t| t.id != id);
        if self.selected_thread.as_deref() == Some(id) {
            self.selected_thread = None;
            self.messages.clear();
            self.investigation_events.clear();
            self.calls.clear();
            self.runs.clear();
            self.answer_memories.clear();
            self.answer_memory_summaries.clear();
            self.input.clear();
            self.recon_chat = false;
        }
        let _ = self.refresh_threads();
        self.reload_memories();
    }

    fn handle_key(&mut self, key: KeyEvent) -> bool {
        if key.code == KeyCode::Esc && self.brain_detail_target.is_some() {
            self.brain_detail_target = None;
            self.brain_detail_generation = self.brain_detail_generation.wrapping_add(1);
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && matches!(key.code, KeyCode::Char('c') | KeyCode::Char('C')) {
            return self.on_interrupt();
        }
        if ctrl && matches!(key.code, KeyCode::Char('q') | KeyCode::Char('Q')) {
            return self.arm_quit();
        }
        if ctrl && matches!(key.code, KeyCode::Char('k') | KeyCode::Char('K')) {
            self.open_palette();
            return true;
        }
        if (self.overlay == Overlay::Configs
            || (self.module == Some(ModuleId::System)
                && self.profile.tab == super::profile::SystemTab::Configs
                && self.overlay == Overlay::None))
            && self.profile_config_key(key)
        {
            return true;
        }
        if self.module == Some(ModuleId::System)
            && self.overlay == Overlay::None
            && super::profile::handle_key(self, key)
        {
            return true;
        }
        if self.module == Some(ModuleId::Brain)
            && self.overlay == Overlay::None
            && self.brain_list_mode == BrainListMode::Topology
            && self.handle_topology_key(key)
        {
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
                    super::ui::reveal_spaced_lists(self);
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    let last = self.filtered_fallback_models().len().saturating_sub(1);
                    self.fallback_popup_sel = (self.fallback_popup_sel + 1).min(last);
                    super::ui::reveal_spaced_lists(self);
                }
                KeyCode::PageUp => super::ui::page(self, -1),
                KeyCode::PageDown => super::ui::page(self, 1),
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
                KeyCode::Down | KeyCode::Tab => {
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
                    let selected = self
                        .palette_items()
                        .get(self.palette_sel)
                        .map(|item| item.id.clone());
                    self.palette_query.pop();
                    self.palette_sel = selected
                        .and_then(|id| self.palette_items().iter().position(|item| item.id == id))
                        .unwrap_or(0);
                }
                KeyCode::Char(c) if !c.is_control() => {
                    let selected = self
                        .palette_items()
                        .get(self.palette_sel)
                        .map(|item| item.id.clone());
                    self.palette_query.push(c);
                    self.palette_sel = selected
                        .and_then(|id| self.palette_items().iter().position(|item| item.id == id))
                        .unwrap_or(0);
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
        if !ctrl && matches!(key.code, KeyCode::Left | KeyCode::Right) {
            if let Target::ToolCategory(category) = self.focus {
                if key.code == KeyCode::Left {
                    self.tool_expanded.remove(category);
                } else {
                    self.tool_expanded.insert(category);
                }
                return true;
            }
            if matches!(
                self.focus,
                Target::PaneTab(_)
                    | Target::ProfileTab(_)
                    | Target::ProfileApp(_)
                    | Target::ProviderTab(_)
                    | Target::IntelTab(_)
            ) {
                let order = super::ui::focus_order(self);
                let same_row = |target: &Target| {
                    std::mem::discriminant(target) == std::mem::discriminant(&self.focus)
                };
                let row: Vec<_> = match self.focus {
                    Target::ProfileApp(_) => (0..6).map(Target::ProfileApp).collect(),
                    Target::ProfileTab(_) => (0..3).map(Target::ProfileTab).collect(),
                    _ => order.into_iter().filter(same_row).collect(),
                };
                if let Some(pos) = row.iter().position(|target| *target == self.focus) {
                    let next = pos
                        .saturating_add_signed(if key.code == KeyCode::Left { -1 } else { 1 })
                        .min(row.len() - 1);
                    self.set_focus(row[next]);
                }
                return true;
            }
        }
        if key.code == KeyCode::Char(' ')
            && matches!(
                self.focus,
                Target::ToolCategory(_)
                    | Target::PaneTab(_)
                    | Target::ProfileTab(_)
                    | Target::ProfileApp(_)
                    | Target::ProviderTab(_)
                    | Target::IntelTab(_)
            )
        {
            self.activate_target(self.focus);
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
            KeyCode::BackTab => self.focus_next(true),
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
        if matches!(self.module, Some(ModuleId::Recon) | Some(ModuleId::Intel))
            && self.brain_destination_back.is_some()
            && (self.module == Some(ModuleId::Recon) || self.related_back.is_empty())
        {
            let snapshot = self.brain_destination_back.take().unwrap();
            if self.module == Some(ModuleId::Recon) {
                self.flush_draft();
            }
            self.module = Some(ModuleId::Brain);
            self.launcher_sel = ModuleId::Brain.index();
            if self.background_db.is_some() {
                self.request_brain_detail(&snapshot.memory_id);
                self.brain_detail_restore = Some(snapshot);
                return;
            }
            let restored = if snapshot.memory_id.starts_with("actor-group:") {
                let depth = self.brain_detail.history.len();
                let opened = self.open_actor_group(&snapshot.memory_id);
                self.brain_detail.history.truncate(depth);
                if opened {
                    self.brain_detail
                        .memory
                        .clone()
                        .map(|memory| (memory, self.brain_graph.clone()))
                } else {
                    None
                }
            } else {
                self.store
                    .get_memory(&snapshot.memory_id)
                    .ok()
                    .flatten()
                    .filter(|m| self.store.is_browse_claim(&m.id).unwrap_or(false))
                    .map(|memory| {
                        let graph = self.store.graph_for_memory(&memory.id).unwrap_or_default();
                        (memory, graph)
                    })
            };
            if let Some((memory, graph)) = restored {
                self.show_detail(memory, graph, Some(&snapshot));
            } else {
                self.leave_brain_detail();
                self.status = "Previous Brain record is unavailable".into();
                return;
            }
            self.status = "Back".into();
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
        if self.module == Some(ModuleId::Brain) && self.brain_list_mode == BrainListMode::Topology {
            self.activate_button(ButtonId::TopologyBack);
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
            Target::Button(ButtonId::DefaultRole(role)) => {
                let index = DefaultsRole::ALL
                    .iter()
                    .position(|r| *r == role)
                    .unwrap_or(0);
                let next = index
                    .saturating_add_signed(delta as isize)
                    .min(DefaultsRole::ALL.len() - 1);
                self.set_focus(Target::Button(ButtonId::DefaultRole(
                    DefaultsRole::ALL[next],
                )));
            }
            Target::Button(ButtonId::FallbackItem(index)) => {
                let count = self
                    .settings
                    .defaults
                    .role(self.defaults_role.role_key())
                    .map_or(0, |role| role.fallbacks.len());
                let next = index
                    .saturating_add_signed(delta as isize)
                    .min(count.saturating_sub(1));
                self.set_focus(Target::Button(ButtonId::FallbackItem(next)));
            }
            Target::Memory(_) => self.move_memory(delta),
            Target::Tool(_) | Target::ToolCategory(_) => self.move_tool(delta),
            Target::LogLine(_) => self.move_log(delta),
            Target::JobRow(_) => self.move_job(delta),
            Target::ReconContext => super::ui::shift_recon_context(self, delta * 3),
            Target::JobDetail => {
                self.jobs.detail_scroll = add_scroll(self.jobs.detail_scroll, delta)
                    .min(super::jobs::detail_scroll_max(self));
            }
            Target::IntelRelatedArticle(index) => {
                let next = (index as i32 + delta).clamp(
                    0,
                    self.intel_related.page.rows.len().saturating_sub(1) as i32,
                ) as usize;
                self.intel_related.sel = next;
                super::ui::reveal_intel_related(self, next);
                self.set_focus(Target::IntelRelatedArticle(next));
            }
            Target::IntelLeftColumn => {
                let max = super::ui::intel_extracted_scroll_max(self);
                self.scrolls.intel_extracted =
                    add_scroll(self.scrolls.intel_extracted, delta * 3).min(max);
            }
            Target::IntelFullArticle => {
                let max = super::ui::intel_full_scroll_max(self);
                let before = self.scrolls.intel_full;
                self.scrolls.intel_full = add_scroll(before, delta * 3).min(max);
                if self.scrolls.intel_full == before {
                    self.scrolls.intel_brief = add_scroll(self.scrolls.intel_brief, delta * 3)
                        .min(super::ui::intel_brief_scroll_max(self));
                }
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
                    self.scrolls.detail = add_scroll(self.scrolls.detail, delta * 3)
                        .min(super::ui::provider_catalog_scroll_max(self));
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
        if self.background_db.is_some() {
            self.pending_job = None;
            let index =
                (self.jobs.sel as i32 + delta).clamp(0, self.jobs.rows.len() as i32 - 1) as usize;
            self.jobs.select_cached(index);
            self.reload_jobs();
        } else {
            self.jobs.move_by(delta, &self.store);
        }
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
        let rows = super::tool_catalog::rows(&self.osint_search, &self.tool_expanded);
        if rows.is_empty() {
            return;
        }
        let pos = rows
            .iter()
            .position(|(target, _)| *target == self.focus)
            .unwrap_or(0);
        let next = pos
            .saturating_add_signed(delta as isize)
            .min(rows.len() - 1);
        let target = rows[next].0;
        match target {
            Target::Tool(index) => self.select_tool(index),
            _ => self.set_focus(target),
        }
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
        if let Some(id) = &self.selected_thread {
            if self
                .store
                .save_draft(
                    id,
                    &self.input,
                    i64::try_from(self.scrolls.chat).unwrap_or(i64::MAX),
                )
                .is_ok()
            {
                self.draft_dirty = false;
            }
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
        let state = HomeDraftState {
            prompt: self.home_draft.clone(),
            cursor: self.home_draft_cursor,
            scroll: self.home_draft_scroll,
            report_mode: self.home_draft_report_mode,
        };
        let json = serde_json::to_string(&state)?;
        self.store.app_state_set("home_recon_draft", &json)?;
        self.home_draft_dirty = false;
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
        && super::ui::intel_insights_loading(app)
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
    app.article_images.detect();
    // Durable local workers (index outbox + summary flush), each claiming only
    // its own pool. Stopped when dropped at the end of the session.
    let _workers = argos_osint_core::scheduler::WorkerPool::spawn_default(paths::db_path());
    // Process liveness for the job registry: jobs left running by a process
    // that exited become interrupted; cross-process Cancel reaches this one.
    let _beat = argos_osint_core::job_registry::start_process_beat(&paths::db_path()).ok();
    // One-time retention cleanup is separate from the elected, resumable
    // backfill pool. Neither belongs to startup or the terminal event loop.
    let repair_tx = app.work_tx.clone();
    let repair_db = paths::db_path();
    std::thread::spawn(move || {
        let outcome = (|| -> Result<()> {
            let store = Store::open_light(&repair_db)?;
            store.prune_events(super::logs::RETENTION_HOURS)?;
            store.clear_old_jobs()?;
            Ok(())
        })()
        .map_err(|e| e.to_string());
        let _ = repair_tx.send(WorkEvent::BrainMaintenance { outcome });
    });
    // Committed memory changes from any writer (Atlas, Recon, Intel Recon, the
    // index/repair workers, other processes) reload Brain, coalesced per poll.
    let mut memory_watch = argos_osint_core::store::MemoryChangeWatcher::new(&app.store);
    let mut memory_polled = std::time::Instant::now();
    let mut dirty = true;
    // Set when the last draw painted a loading animation (see `anim`).
    let mut animated = false;
    loop {
        if pump(&mut app) {
            dirty = true;
        }
        if app.tick_dashboards() {
            dirty = true;
        }
        if memory_polled.elapsed() >= Duration::from_secs(1) {
            memory_polled = std::time::Instant::now();
            if app.module == Some(ModuleId::Intel) && app.intel_page == IntelPage::Briefing {
                let previous = app.intel_refresh_status.clone();
                app.refresh_intel_briefing();
                if previous != app.intel_refresh_status {
                    dirty = true;
                }
            }
            if memory_watch.poll(&app.store).is_some() {
                app.reload_memories();
                if app.module == Some(ModuleId::Intel) && app.intel_page == IntelPage::Briefing {
                    app.refresh_intel_briefing();
                }
                dirty = true;
            }
        }
        let image_area = super::ui::article_image_area(&app);
        let image_article = if app.module == Some(ModuleId::Intel) && app.overlay == Overlay::None {
            app.intel_articles
                .get(app.intel_sel)
                .map(|article| (article.id.as_str(), article.image_url.as_str()))
        } else {
            None
        };
        if app.article_images.update(image_article, image_area) {
            dirty = true;
        }
        if app.article_images.clear_graphics {
            use std::io::Write;
            // Kitty delete-all is ignored by other protocols; clearing the cell
            // buffer also erases Sixel/iTerm and half-block leftovers.
            let _ = std::io::stdout().write_all(b"\x1b_Ga=d,d=A;\x1b\\");
            terminal.clear()?;
            app.article_images.clear_graphics = false;
        }
        if dirty {
            if let Ok(size) = terminal.size() {
                app.screen = Rect::new(0, 0, size.width, size.height);
            }
            super::ui::normalize(&mut app);
            super::anim::take_animated();
            terminal.draw(|frame| {
                app.screen = frame.area();
                super::ui::draw(frame, &app)
            })?;
            animated = super::anim::take_animated();
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
        if animated
            || atlas_extracting_visible(&app)
            || intel_body_loading_visible(&app)
            || intel_insights_loading_visible(&app)
        {
            wait = wait.min(Duration::from_millis(80));
        }
        if !event::poll(wait)? {
            if app.module.is_none() && app.advance_recon_tip(Instant::now()) {
                dirty = true;
            }
            if app.module == Some(ModuleId::Recon)
                && app.focus == Target::Field(FieldId::Composer)
                && app.draft_dirty
            {
                app.flush_draft();
            }
            if composer_focused
                || animated
                || atlas_countdown_visible(&app)
                || atlas_extracting_visible(&app)
                || intel_body_loading_visible(&app)
                || intel_insights_loading_visible(&app)
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
    let started = Instant::now();
    for _ in 0..32 {
        let Ok(message) = app.provider_rx.try_recv() else {
            break;
        };
        app.on_provider_event(message);
        dirty = true;
        if started.elapsed() >= Duration::from_millis(8) {
            break;
        }
    }
    for _ in 0..32 {
        let Ok(message) = app.work_rx.try_recv() else {
            break;
        };
        dirty |= app.on_work_event(message);
        if started.elapsed() >= Duration::from_millis(8) {
            break;
        }
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

/// Button/title label for the Brain kind filter.
pub(crate) fn association_rows(
    store: &Store,
    id: &str,
    tab: RelatedTab,
) -> Result<Vec<AssociationItem>> {
    let a = if id.starts_with("actor-group:") {
        let g = store
            .actor_group(id)?
            .ok_or_else(|| anyhow::anyhow!("actor unavailable"))?;
        store.actor_group_associations(&g)?
    } else {
        store.claim_associations(id)?
    };
    Ok(association_rows_from(a, tab))
}

fn association_rows_from(
    a: argos_osint_core::brain_browse::Associations,
    tab: RelatedTab,
) -> Vec<AssociationItem> {
    let row = |memory_id: String, title: String, reason: &str, target: AssociationTarget| {
        AssociationItem {
            key: memory_id,
            title,
            reason: reason.into(),
            target,
        }
    };
    let mut rows = Vec::new();
    match tab {
        RelatedTab::Claims => {
            for m in a.claims {
                rows.push(row(
                    m.id.clone(),
                    m.text,
                    "claim references this name group",
                    AssociationTarget::Claim(m.id),
                ));
            }
        }
        RelatedTab::Actors => {
            for g in a.actors {
                rows.push(row(
                    g.key(),
                    g.display_name.clone(),
                    "referenced actor",
                    AssociationTarget::ActorGroup(g.key()),
                ));
            }
        }
        RelatedTab::Threads | RelatedTab::Sources => {
            for (id, title) in a.threads {
                rows.push(row(
                    format!("thread:{id}"),
                    title,
                    "origin investigation",
                    AssociationTarget::Thread(id),
                ));
            }
        }
        RelatedTab::Articles => {}
    }
    if matches!(tab, RelatedTab::Articles | RelatedTab::Sources) {
        for a in a.articles {
            rows.push(row(
                format!("article:{}/{}", a.article.run_id, a.article.id),
                a.article.title.clone(),
                "retained source article",
                AssociationTarget::Article {
                    run_id: a.article.run_id.clone(),
                    article_id: a.article.id.clone(),
                    revision: argos_osint_core::atlas_metadata::input_hash(&a.article),
                },
            ));
        }
    }
    rows
}

fn load_brain_detail(store: &Store, id: &str) -> Result<LoadedBrainDetail> {
    if id.starts_with("actor-group:") {
        let g = store
            .actor_group(id)?
            .ok_or_else(|| anyhow::anyhow!("actor unavailable"))?;
        let associations = store.actor_group_associations(&g)?;
        let graph = Store::actor_group_graph_from(&g, associations.clone());
        let memory = Memory {
            id: g.key(),
            text: g.display_name,
            category: "actor group".into(),
            pinned: false,
            created_at: String::new(),
            source: MemorySource {
                app: "brain-actor-group".into(),
                conversation_id: g.name_key,
                message_id: None,
                reference: serde_json::to_string(&g.actor_ids).ok(),
            },
        };
        Ok(LoadedBrainDetail {
            memory,
            graph,
            typed: Vec::new(),
            insight: None,
            claim: false,
            associations,
        })
    } else {
        anyhow::ensure!(store.is_browse_claim(id)?, "claim unavailable");
        let memory = store
            .get_memory(id)?
            .ok_or_else(|| anyhow::anyhow!("claim unavailable"))?;
        Ok(LoadedBrainDetail {
            graph: store.graph_for_memory(id)?,
            typed: argos_osint_core::brain_kinds::typed_detail(store, id)?,
            insight: store.insight_for_memory(id)?,
            claim: store
                .memory_insight_kind(id)
                .is_none_or(|kind| kind == "claim"),
            associations: store.claim_associations(id)?,
            memory,
        })
    }
}

pub(crate) fn brain_kind_label(
    kind: Option<argos_osint_core::memory_metadata::MemoryKind>,
) -> &'static str {
    match kind {
        None => "All",
        Some(argos_osint_core::memory_metadata::MemoryKind::Claim) => "Claims",
        Some(argos_osint_core::memory_metadata::MemoryKind::Actor) => "Actors",
        _ => "Unavailable",
    }
}

/// Path-pane title for Actor/Link/Related details (`None` for claim/recon).
pub(crate) fn structural_title(app: &App) -> Option<&'static str> {
    if app
        .brain_detail
        .memory_id()
        .is_some_and(|id| id.starts_with("actor-group:"))
    {
        return Some(" actor group ");
    }
    match app.brain_typed.first().map(String::as_str) {
        Some("Kind: Actor") => Some(" actor graph "),
        Some("Kind: Link") => Some(" link graph "),
        Some("Kind: Related") => Some(" related lead "),
        _ => None,
    }
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
            recon_tip: 0,
            recon_tip_at: Instant::now(),
            input: String::new(),
            brain_app: String::new(),
            brain_conversation: String::new(),
            brain_insight: String::new(),
            brain_query: String::new(),
            recon_search: String::new(),
            intel_search: String::new(),
            osint_search: String::new(),
            tool_expanded: crate::tui::tool_catalog::initial_expansion(),
            response_scroll: 0,
            intel_preview_scroll: 0,
            tool_dataset_status: String::new(),
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
            investigation_events: Vec::new(),
            compact_pages: [0; 9],
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
            answer_memory_summaries: HashMap::new(),
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
            brief_op_sel: 0,
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
            article_images: crate::tui::article_image::ArticleImages::default(),
            palette_recent: Vec::new(),
            palette_restore: None,
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
            intel_body_read_error: None,
            intel_body_owner: None,
            intel_body_require_commit: false,
            intel_full_collapsed: false,
            intel_body_running: HashSet::new(),
            intel_brief_job: None,
            intel_brief_stages: Vec::new(),
            intel_brief_outputs: HashMap::new(),
            intel_brief_lead_titles: HashMap::new(),
            intel_brief_publication: String::new(),
            intel_insights_running: HashSet::new(),
            intel_links_reviewing: HashSet::new(),
            intel_insight_lifecycle: "legacy/unassessed".into(),
            intel_extraction_progress: None,
            intel_link_explanations: HashMap::new(),
            intel_actors: Vec::new(),
            intel_actor_state: "pending".into(),
            intel_refresh_status: "Waiting".into(),
            intel_evidence_confidence: None,
            intel_metadata_present: false,
            intel_publisher_prior: None,
            intel_related: Default::default(),
            related_back: Vec::new(),
            brain_destination_back: None,
            brain_graph: recon::MemoryGraph::default(),
            brain_typed: Vec::new(),
            brain_kind: None,
            related_scope: RelatedScope::default(),
            brain_graph_for: None,
            brain_detail: Default::default(),
            topology_state: Default::default(),
            topology_origin: None,
            topology_generation: 0,
            topology_reading: false,
            topology_loading: false,
            topology_error: None,
            topology_origin_label: String::new(),
            memory_total: 0,
            memory_error: None,
            memories_loaded: true,
            background_db: None,
            atlas_deleting: None,
            atlas_reading: false,
            atlas_generation: 0,
            intel_reading: false,
            intel_related_limit: 20,
            dashboard_reading: false,
            pending_job: None,
            jobs_clearing: false,
            logs_clearing: false,
            threads_deleting: HashSet::new(),
            memories_deleting: HashSet::new(),
            recon_reading: false,
            threads_reading: false,
            intel_list_reading: false,
            atlas_articles_reading: false,
            manual_reading: false,
            manual_pending: false,
            threads_pending: false,
            recon_pending: false,
            intel_list_pending: false,
            brain_preview_reading: false,
            brain_preview_target: None,
            brain_preview_for: None,
            brain_cache: Vec::new(),
            brain_generation: 0,
            brain_reading: None,
            brain_dirty: false,
            brain_prefer: None,
            brain_detail_generation: 0,
            brain_detail_reading: false,
            brain_detail_target: None,
            brain_detail_restore: None,
            brain_detail_back: false,
            brain_associations: None,
            brain_detail_claim: true,
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
        assert!(app
            .brain_graph
            .nodes
            .iter()
            .any(|n| n.kind == recon::GraphNodeKind::Finding));
        assert!(app.graph_summary.contains("Atlas launch"));
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
            "Browse classified headlines and run article briefings"
        );
        assert_eq!(
            ModuleId::Atlas.blurb(),
            "Collect and classify global headlines on a live map"
        );
        assert_eq!(
            ModuleId::Brain.blurb(),
            "Explore saved memories, their paths, and related evidence"
        );
        assert_eq!(
            ModuleId::Recon.blurb(),
            "Run evidence-driven OSINT investigations on any target"
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
        assert_eq!(app.overlay, Overlay::None);
        assert_eq!(app.focus, Target::IntelPreview);
        app.on_esc();
        assert_eq!(app.overlay, Overlay::None);
        assert_eq!(app.intel_page, IntelPage::Bulletin);
    }

    #[test]
    fn brain_thread_association_back_restores_the_original_group_and_tab() {
        let mut app = app();
        app.screen = Rect::new(0, 0, 140, 40);
        brain_world(&app);
        let thread = app.store.new_thread("Ana Ruiz investigation").unwrap();
        app.store.set_recall_insights(&thread.id, true).unwrap();
        app.store
            .record_thread_actor(&thread.id, "Ana Ruiz", "person")
            .unwrap();
        let ids = claim_fixture(&app);
        app.select(ModuleId::Brain.index());
        assert!(app.open_actor_group("actor-group:ana ruiz"));
        app.select_brain_related_tab(1);
        app.brain_detail.related.page_sizes[1] = 40;
        app.set_focus(Target::RelatedRow(0));
        app.scrolls.path = 3;
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.module, Some(ModuleId::Recon));
        assert_eq!(app.selected_thread.as_deref(), Some(thread.id.as_str()));
        // A visit to another Brain detail must not replace the saved launcher.
        assert!(app.open_memory_detail(&ids[0]));
        app.module = Some(ModuleId::Recon);
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.module, Some(ModuleId::Brain));
        assert_eq!(app.brain_detail.memory_id(), Some("actor-group:ana ruiz"));
        assert_eq!(app.brain_detail.related.tab, RelatedTab::Threads);
        assert_eq!(app.brain_detail.related.page_sizes[1], 40);
        assert_eq!(app.scrolls.path, 3);
        assert_eq!(app.focus, Target::RelatedRow(0));
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
        app.compact_pages[ModuleId::Providers.index()] = 1;
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
        click(&mut app, Target::PaneTab(1));
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
        click(&mut app, Target::PaneTab(1));
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

        click(&mut app, Target::PaneTab(0));
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
        assert!(!hit(&app, Target::Button(ButtonId::CancelRun)));
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
        app.compact_pages[ModuleId::Osint.index()] = 2;
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
        app.profile.tab = super::super::profile::SystemTab::System;
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
            "Usage analytics, host details, and config transfer"
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
        let mut text = buffer_text(&render(&mut app2, 160, 40));
        super::super::ui::page(&mut app2, 1);
        text.push_str(&buffer_text(&render(&mut app2, 160, 40)));
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
        assert!(text.contains("Stop job"), "{text}");
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
        assert!(app.status.contains("Stop requested"), "{}", app.status);
        assert_eq!(
            app.jobs.selected().unwrap().state,
            "running",
            "not cancelled until it stops"
        );
        let text = buffer_text(&render(&mut app, 140, 40));
        assert!(text.contains("Stopping…"), "{text}");
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
    fn jobs_status_controls_and_service_screens() {
        use argos_osint_core::jobs_view::{JobDetail, JobRow, JobStatusFilter};
        let mut app = app();
        for (id, state) in [
            ("running", "running"),
            ("completed", "completed"),
            ("stopped", "cancelled"),
            ("failed", "failed"),
        ] {
            register(&app, id, "brain", &format!("Fixture {id} job"));
            app.store
                .set_job_progress(id, state, "fixture", 0, None, "")
                .unwrap();
        }
        app.store
            .fixture_task_attempt("running", "running-task", "summarization", 90, None)
            .unwrap();
        app.select(ModuleId::Jobs.index());
        let dir = std::env::var("ARGOS_SCREEN_DIR").ok();
        let clears = [
            ButtonId::JobsClearCompleted,
            ButtonId::JobsClearStopped,
            ButtonId::JobsClearFailed,
        ];
        for (width, height) in [
            (160, 50),
            (120, 40),
            (100, 32),
            (80, 24),
            (60, 18),
            (40, 20),
        ] {
            let buffer = render(&mut app, width, height);
            let text = buffer_text(&buffer);
            for label in [
                "Clear completed",
                "Clear stopped",
                "Clear failed",
                "Stop job",
            ] {
                assert!(
                    text.contains(label),
                    "{width}x{height}: missing {label}\n{text}"
                );
            }
            assert!(!text.contains("Clear old jobs"));
            let order = super::super::ui::focus_order(&app);
            for button in clears.into_iter().chain([ButtonId::JobsCancel]) {
                let target = Target::Button(button);
                assert!(
                    order.contains(&target),
                    "unreachable {button:?} at {width}x{height}"
                );
                let rect = app
                    .layout
                    .borrow()
                    .entries
                    .iter()
                    .find(|e| e.target == target)
                    .unwrap()
                    .rect;
                assert_eq!(
                    super::super::ui::hit_test(&app, rect.x, rect.y),
                    Some(target)
                );
            }
            if let Some(dir) = &dir {
                dump_cells(
                    &mut app,
                    dir,
                    &format!("jobs-running-{width}x{height}"),
                    width,
                    height,
                );
            }
        }
        let running = app.jobs.selected().unwrap().clone();
        for state in ["queued", "retry_scheduled"] {
            let mut job = running.clone();
            job.state = state.into();
            app.jobs.rows = vec![job.clone()];
            app.jobs.sel = 0;
            app.jobs.detail = Some(JobDetail {
                job,
                ..Default::default()
            });
            let text = buffer_text(&render(&mut app, 80, 24));
            assert!(app.jobs.can_cancel() && text.contains("Stop job"));
            if let Some(dir) = &dir {
                dump_cells(&mut app, dir, &format!("jobs-{state}-80x24"), 80, 24);
            }
        }
        let service = JobRow {
            id: "service".into(),
            kind: "service".into(),
            app: "brain".into(),
            title: "Summary flush worker".into(),
            state: "running".into(),
            ..Default::default()
        };
        app.jobs.rows = vec![service.clone()];
        app.jobs.sel = 0;
        app.jobs.detail = Some(JobDetail {
            job: service,
            ..Default::default()
        });
        app.jobs.detail_open = true;
        for (width, height) in [(160, 50), (80, 24), (60, 18)] {
            let text = buffer_text(&render(&mut app, width, height));
            assert!(!app.jobs.can_cancel() && !text.contains("Stop job"));
            assert!(text.contains("Long-lived service"));
            if let Some(dir) = &dir {
                dump_cells(
                    &mut app,
                    dir,
                    &format!("jobs-service-{width}x{height}"),
                    width,
                    height,
                );
            }
        }
        app.jobs = Default::default();
        for state in ["loading", "empty", "error"] {
            app.jobs.loaded = state != "loading";
            app.jobs.error = (state == "error").then(|| "Fixture retained read failure".into());
            if state == "error" {
                app.jobs.rows = vec![running.clone()];
            }
            if let Some(dir) = &dir {
                dump_cells(&mut app, dir, &format!("jobs-{state}-80x24"), 80, 24);
            }
        }
        app.jobs.reload(&app.store);
        app.overlay = Overlay::Help;
        for (width, height) in [(80, 24), (60, 18)] {
            if let Some(dir) = &dir {
                dump_cells(
                    &mut app,
                    dir,
                    &format!("jobs-help-{width}x{height}"),
                    width,
                    height,
                );
            }
        }
        app.overlay = Overlay::None;
        app.screen = Rect::new(0, 0, 160, 50);
        for (button, id) in clears.into_iter().zip(["completed", "stopped", "failed"]) {
            click(&mut app, Target::Button(button));
            assert!(app.store.get_job(id).unwrap().is_none());
            assert!(app.store.get_job("running").unwrap().is_some());
        }
        click(&mut app, Target::Button(ButtonId::JobsCancel));
        assert_eq!(
            app.store.get_job("running").unwrap().unwrap().state,
            "cancelled"
        );
        app.jobs.status = JobStatusFilter::Stopped;
        app.reload_jobs();
        assert_eq!(app.jobs.rows.len(), 1);
        assert_eq!(app.jobs.rows[0].id, "running");
        click(&mut app, Target::Button(ButtonId::JobsClearStopped));
        assert!(app.jobs.rows.is_empty() && app.jobs.detail.is_none());
    }

    #[test]
    fn jobs_stop_targets_selected_job_only() {
        for keyboard in [false, true] {
            for selected_state in ["running", "queued", "retry_scheduled"] {
                let mut app = app();
                app.screen = Rect::new(0, 0, 160, 50);
                for (id, state) in [
                    ("other-running", "running"),
                    ("selected", selected_state),
                    ("other-queued", "queued"),
                ] {
                    register(&app, id, "brain", id);
                    app.store
                        .fixture_task_attempt(id, &format!("{id}-task"), "summarization", 90, None)
                        .unwrap();
                    app.store
                        .set_job_progress(id, state, "fixture", 0, None, "")
                        .unwrap();
                }
                app.select(ModuleId::Jobs.index());
                let selected = app
                    .jobs
                    .rows
                    .iter()
                    .position(|job| job.id == "selected")
                    .unwrap();
                click(&mut app, Target::JobRow(selected));
                // A refresh must retain the selected identity, independent of row order.
                app.reload_jobs();
                assert_eq!(app.jobs.selected().unwrap().id, "selected");
                if keyboard {
                    app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE));
                } else {
                    click(&mut app, Target::Button(ButtonId::JobsCancel));
                }
                assert_eq!(
                    app.store.get_job("selected").unwrap().unwrap().state,
                    "cancelled"
                );
                for (id, state) in [("other-running", "running"), ("other-queued", "queued")] {
                    let job = app.store.get_job(id).unwrap().unwrap();
                    assert_eq!(job.state, state);
                    assert!(!job.cancel_requested);
                    assert_eq!(
                        app.store.job_detail(id, 40).unwrap().unwrap().tasks[0].state,
                        "running"
                    );
                }
            }
        }
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
            overview.contains("[p]") && overview.contains("Overview"),
            "the overview tab shows the filter strip and the tab switch: {overview}"
        );
        assert!(
            !overview.contains("Database:") && !overview.contains("Refresh hardware"),
            "host and paths belong to the System tab: {overview}"
        );

        // A bare Tab moves to the System tab, which keeps the panes it always had.
        app.handle_key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE));
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
        app.handle_key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE));
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
    fn claim_detail_has_graph_above_actors_and_summary_and_restores_navigation() {
        let mut app = app();
        app.screen = Rect::new(0, 0, 140, 40);
        let ids = claim_fixture(&app);
        app.select(ModuleId::Brain.index());
        assert!(app.open_memory_detail(&ids[0]));
        let areas = super::super::ui::detail_areas(&app);
        assert!(!areas.stacked);
        assert!(areas.path.y < areas.related.y);
        assert_eq!(areas.related.y, areas.summary.y);
        assert_eq!(app.brain_detail.related.tab, RelatedTab::Actors);
        let items = app.brain_detail.related.items.clone();
        assert!(!items.is_empty());
        assert!(items
            .iter()
            .all(|i| matches!(i.target, AssociationTarget::ActorGroup(_))));
        render(&mut app, 140, 40);
        click(&mut app, Target::RelatedRow(0));
        assert!(app
            .brain_detail
            .memory_id()
            .unwrap()
            .starts_with("actor-group:"));
        assert_eq!(app.brain_detail.related.tab, RelatedTab::Claims);
        app.activate_button(ButtonId::Pin);
        app.activate_button(ButtonId::Delete);
        assert!(app.status.contains("read-only"));
        assert!(app
            .store
            .get_memory(&ids[0])
            .unwrap()
            .is_some_and(|m| !m.pinned));
        let generation = app.brain_detail.related.request;
        assert!(!app.on_related(generation - 1, &ids[0], Ok(Vec::new())));
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.brain_detail.memory_id(), Some(ids[0].as_str()));
        assert_eq!(app.brain_detail.related.tab, RelatedTab::Actors);
        app.select_brain_related_tab(1);
        assert!(app.brain_detail.related.items.iter().all(|i| matches!(
            i.target,
            AssociationTarget::Article { .. } | AssociationTarget::Thread(_)
        )));
        render(&mut app, 140, 40);
        click(&mut app, Target::RelatedRow(0));
        assert_eq!(app.module, Some(ModuleId::Intel));
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.module, Some(ModuleId::Brain));
        assert_eq!(app.brain_detail.related.tab, RelatedTab::Sources);
        assert_eq!(app.brain_detail.memory_id(), Some(ids[0].as_str()));
        assert_eq!(app.focus, Target::RelatedRow(0));
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
        assert!(areas.related.height >= 3 && areas.summary.height >= 3);
        app.set_focus(Target::DetailSummary);
        let focused = super::super::ui::detail_areas(&app);
        assert!(focused.summary.height > focused.related.height);
        let text = buffer_text(&render(&mut app, 64, 32));
        assert!(text.contains("Claim") && text.contains("Source"), "{text}");
        click(&mut app, Target::RelatedRow(0));
        assert!(app
            .brain_detail
            .memory_id()
            .unwrap()
            .starts_with("actor-group:"));
        click(&mut app, Target::Button(ButtonId::BrainDetailBack));
        assert_eq!(app.brain_detail.memory_id(), Some(ids[0].as_str()));
    }

    #[test]
    fn brain_refresh_keeps_selection_and_find_and_shows_read_errors() {
        let mut app = app();
        app.screen = Rect::new(0, 0, 120, 34);
        let add = |app: &App, text: &str| {
            let memory = app
                .store
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
                .unwrap();
            app.store.mark_user_claim(&memory.id).unwrap();
            memory.id
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
    fn atlas_history_resume_is_offered_only_for_resumable_cycles_without_repair() {
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
            ButtonId::AtlasDelete,
        ] {
            assert!(order.contains(&Target::Button(button)), "{button:?}");
        }
        let text = buffer_text(&render(&mut app, 140, 44));
        assert!(text.contains("Resume"), "{text}");
        assert!(!text.contains("Repair memories"), "{text}");
        assert!(!super::super::commands::COMMANDS
            .iter()
            .any(|command| command.id == "atlas-repair"));
    }

    fn related_ui_fixture(app: &mut App) -> (String, AtlasArticleRow) {
        let ids = claim_fixture(app);
        brain_world(app);
        let article = app.store.atlas_article("run-1012", "k1").unwrap().unwrap();
        let page = app
            .store
            .related_articles_global(
                &article.run_id,
                &article.id,
                RelatedScope::AllRetained,
                None,
                20,
            )
            .unwrap()
            .into_page(0);
        assert!(app.open_memory_detail(&ids[0]));
        app.brain_detail.related.tab = RelatedTab::Articles;
        app.brain_detail.related.articles.page = page;
        app.compact_pages[ModuleId::Brain.index()] = 2;
        (ids[0].clone(), article)
    }

    #[test]
    fn intel_related_pages_its_rows_before_the_middle_stack() {
        let mut app = app();
        app.screen = Rect::new(0, 0, 160, 50);
        let (_, article) = related_ui_fixture(&mut app);
        app.open_intel_briefing_for(article).unwrap();
        assert!(
            super::super::ui::focus_order(&app).contains(&Target::Button(ButtonId::RelatedScope)),
            "offscreen Related must be reachable"
        );
        let row = app.brain_detail.related.articles.page.rows[0].clone();
        app.intel_related.page.rows = (0..30).map(|_| row.clone()).collect();
        app.intel_related.page.total = 30;
        app.set_focus(Target::IntelRelatedArticle(0));
        app.intel_related.scroll = 6;
        app.scrolls.intel_brief = 6;
        super::super::ui::page(&mut app, -1);
        assert_eq!(app.intel_related.scroll, 0);
        assert_eq!(app.scrolls.intel_brief, 6, "rows scroll first");
        super::super::ui::page(&mut app, -1);
        assert_eq!(app.intel_related.scroll, 0);
        assert_eq!(app.scrolls.intel_brief, 0, "stack scrolls at row boundary");
    }

    #[test]
    fn intel_related_back_restores_the_original_full_article_offset() {
        let mut app = app();
        app.screen = Rect::new(0, 0, 160, 50);
        let (_, article) = related_ui_fixture(&mut app);
        app.open_intel_briefing_for(article.clone()).unwrap();
        let related = app.brain_detail.related.articles.clone();
        assert!(!related.page.rows.is_empty());
        app.intel_related = related;
        app.scrolls.intel_full = 17;
        app.open_related_article(0);
        assert_eq!(app.scrolls.intel_full, 0);
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.intel_articles[app.intel_sel].id, article.id);
        assert_eq!(app.scrolls.intel_full, 17);
        assert_eq!(app.focus, Target::IntelRelatedArticle(0));
    }

    #[test]
    fn related_article_navigation_checks_exact_identity_and_restores_memory() {
        let mut app = app();
        app.screen = Rect::new(0, 0, 120, 40);
        let ids = claim_fixture(&app);
        assert!(app.open_memory_detail(&ids[0]));
        app.module = Some(ModuleId::Brain);
        app.select_brain_related_tab(1);
        let item = app.brain_detail.related.items[0].clone();
        let AssociationTarget::Article {
            run_id, article_id, ..
        } = item.target
        else {
            panic!("article source")
        };
        let mut article = app
            .store
            .atlas_article(&run_id, &article_id)
            .unwrap()
            .unwrap();
        article.title = "Replacement revision".into();
        app.store.atlas_upsert_article(&article).unwrap();
        app.brain_detail.related.sel = 0;
        app.open_selected_related();
        assert_eq!(app.module, Some(ModuleId::Brain));
        assert!(app.status.contains("unavailable"));
        app.select_brain_related_tab(1);
        app.set_focus(Target::RelatedRow(0));
        app.open_selected_related();
        assert_eq!(app.module, Some(ModuleId::Intel));
        assert_eq!(app.intel_articles[app.intel_sel].run_id, run_id);
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.module, Some(ModuleId::Brain));
        assert_eq!(app.focus, Target::RelatedRow(0));
    }

    #[test]
    fn dump_atlas_related_screens() {
        let dir = std::env::var("ARGOS_SCREEN_DIR").ok();
        let mut app = app();
        let (_, article) = related_ui_fixture(&mut app);
        for n in 0..42 {
            let mut candidate = article.clone();
            candidate.id = format!("related-overflow-{n}");
            candidate.url = format!("https://news.example/related-overflow-{n}");
            candidate.title=format!("Ana Ruiz announces a long cross-border ferry transport agreement with the Baltic regional ports authority {n}");
            app.store.atlas_upsert_article(&candidate).unwrap();
            let mut metadata =
                argos_osint_core::atlas_metadata::ArticleMetadata::pending(&candidate);
            metadata.entity_state = argos_osint_core::atlas_metadata::MetadataState::Complete;
            for (name, kind) in [("Ana Ruiz", "person"), ("Baltic", "place")] {
                let start = candidate.title.find(name).unwrap();
                metadata
                    .mentions
                    .push(argos_osint_core::atlas_metadata::ArticleMention {
                        type_hint: kind.into(),
                        comparison_key: name.to_lowercase(),
                        field: "title".into(),
                        start_byte: start,
                        end_byte: start + name.len(),
                        original_text: name.into(),
                    });
            }
            app.store.atlas_save_metadata(&metadata).unwrap();
        }
        argos_osint_core::related_articles::build(&app.store, &article.run_id).unwrap();
        app.brain_detail.related.articles.page = app
            .store
            .related_articles_global(
                &article.run_id,
                &article.id,
                RelatedScope::AllRetained,
                None,
                20,
            )
            .unwrap()
            .into_page(0);
        assert!(app.brain_detail.related.articles.page.total > 40);
        assert!(app.brain_detail.related.articles.page.has_more);
        for (width, height) in [
            (160, 50),
            (120, 40),
            (100, 32),
            (80, 24),
            (60, 18),
            (40, 20),
        ] {
            app.screen = Rect::new(0, 0, width, height);
            app.open_intel_briefing_for(article.clone()).unwrap();
            app.intel_related = app.brain_detail.related.articles.clone();
            app.compact_pages[ModuleId::Intel.index()] = 1;
            render(&mut app, width, height);
            super::super::ui::reveal_intel_related(&mut app, 0);
            app.set_focus(Target::IntelRelatedArticle(0));
            let text = buffer_text(&render(&mut app, width, height));
            if width >= 60 {
                assert!(text.contains("Related"), "{text}");
            }
            if let Some(dir) = &dir {
                dump_cells(
                    &mut app,
                    dir,
                    &format!("intel-related-articles-{width}x{height}"),
                    width,
                    height,
                );
            }
            app.overlay = Overlay::Help;
            if let Some(dir) = &dir {
                dump_cells(
                    &mut app,
                    dir,
                    &format!("intel-help-{width}x{height}"),
                    width,
                    height,
                );
            }
            app.overlay = Overlay::None;
            for state in ["empty", "loading", "unavailable"] {
                app.intel_related = Default::default();
                app.intel_related.loading = state == "loading";
                if state == "unavailable" {
                    app.intel_related.error = Some("metadata unavailable".into());
                }
                if let Some(dir) = &dir {
                    dump_cells(
                        &mut app,
                        dir,
                        &format!("intel-related-{state}-{width}x{height}"),
                        width,
                        height,
                    );
                }
            }
            // Keep the pre-existing Atlas Coverage fixture for other reviews.
            app.select(ModuleId::Atlas.index());
            app.atlas_page = AtlasPage::Live;
            app.compact_pages[ModuleId::Atlas.index()] = 1;
            app.atlas_status = "Native metadata settled".into();
            app.atlas_stats.coverage.articles = 12;
            app.atlas_stats.coverage.classified = 12;
            app.atlas_stats.coverage.entities_available = 10;
            app.atlas_stats.coverage.publishers_available = 6;
            app.atlas_stats.coverage.related_pairs = 4;
            if let Some(dir) = &dir {
                dump_cells(
                    &mut app,
                    dir,
                    &format!("atlas-coverage-{width}x{height}"),
                    width,
                    height,
                );
            }
        }
    }

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
        app.brain_detail.related.tab = RelatedTab::Actors;
        // Fixture summary (the test app has no Summarization account).
        app.graph_summary = "## Northwind halted Baltic crossings\n\nTwo **Baltic Wire** articles in cycle 04:12 support the halt. A later board decision **revises** it to a partial suspension, so the claim is a *dated fact*, not current status.\n\n- Evidence: 2 articles, 1 cycle\n- Related: 1 revision, 2 linked claims".into();
        app.status = "Claim path".into();
        app.brain_detail.related.tab = RelatedTab::Actors;
        while !matches!(app.focus, Target::RelatedRow(_)) {
            app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        }
        dump_cells(&mut app, &dir, "brain-claim-detail", 140, 40);
        app.screen = Rect::new(0, 0, 64, 32);
        dump_cells(&mut app, &dir, "brain-claim-detail-narrow", 64, 32);
    }

    #[test]
    fn recon_tips_cover_every_catalog_target_type_once() {
        use argos_osint_core::osint::contracts::IntelligenceCategory as C;
        let kinds: Vec<&str> = RECON_TIPS.iter().map(|(k, _)| *k).collect();
        let mut unique = kinds.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), kinds.len(), "duplicate tip kinds");
        for required in [
            "person",
            "username",
            "email",
            "phone",
            "domain",
            "ip",
            "company",
            "news",
            "legal",
            "social",
            "url",
            "history",
            "code",
            "cve",
            "package",
            "bitcoin",
            "place",
            "publisher",
        ] {
            assert!(kinds.contains(&required), "missing tip for {required}");
        }
        // Every catalog category has at least one tip that exercises it.
        let by_category = |c: C| -> &[&str] {
            match c {
                C::WebDiscovery => &["url", "domain"],
                C::NewsEvents => &["news"],
                C::PublisherContext => &["publisher"],
                C::LegalProceedings => &["legal"],
                C::Organizations => &["company"],
                C::ProfessionalIdentity => &["person", "email"],
                C::SocialContent => &["social"],
                C::PublicAccountCorroboration => &["username"],
                C::DomainNetwork => &["domain", "ip"],
                C::HistoricalWeb => &["history"],
                C::SoftwareCode => &["code"],
                C::Geography => &["place"],
                C::Bitcoin => &["bitcoin"],
                C::Vulnerabilities => &["cve", "package", "ip"],
                C::EmailRegistration => &["email"],
            }
        };
        for c in [
            C::WebDiscovery,
            C::NewsEvents,
            C::PublisherContext,
            C::LegalProceedings,
            C::Organizations,
            C::ProfessionalIdentity,
            C::SocialContent,
            C::PublicAccountCorroboration,
            C::DomainNetwork,
            C::HistoricalWeb,
            C::SoftwareCode,
            C::Geography,
            C::Bitcoin,
            C::Vulnerabilities,
            C::EmailRegistration,
        ] {
            assert!(by_category(c).iter().any(|k| kinds.contains(k)), "{c:?}");
        }
    }

    #[test]
    fn recon_tips_rotate_in_order_on_the_interval_and_wrap() {
        let mut app = app();
        let start = Instant::now();
        app.recon_tip_at = start;
        assert_eq!(app.recon_tip_text(), Some(RECON_TIPS[0].1));
        assert!(!app.advance_recon_tip(start + RECON_TIP_EVERY / 2));
        let mut now = start;
        for step in 1..=RECON_TIPS.len() {
            now += RECON_TIP_EVERY;
            assert!(app.advance_recon_tip(now));
            assert_eq!(app.recon_tip, step % RECON_TIPS.len());
        }
        assert_eq!(app.recon_tip_text(), Some(RECON_TIPS[0].1));
    }

    #[test]
    fn recon_tip_hides_and_freezes_while_typing() {
        let mut app = app();
        let start = Instant::now();
        app.recon_tip_at = start;
        app.input = "who is".into();
        assert_eq!(app.recon_tip_text(), None);
        assert!(!app.advance_recon_tip(start + RECON_TIP_EVERY * 3));
        assert_eq!(app.recon_tip, 0);
        let text = buffer_text(&render(&mut app, 120, 40));
        assert!(!text.contains("Ask anything"), "{text}");
        app.input.clear();
        // Clearing restarts the interval rather than skipping ahead.
        assert!(!app.advance_recon_tip(start + RECON_TIP_EVERY * 3));
        assert!(app.advance_recon_tip(start + RECON_TIP_EVERY * 4));
        assert_eq!(app.recon_tip, 1);
        let text = buffer_text(&render(&mut app, 160, 40));
        assert!(text.contains(RECON_TIPS[1].1), "{text}");
    }

    /// Home with the current app descriptions and several composer tips:
    /// `ARGOS_SCREEN_DIR=... cargo test -p argos-osint-bin dump_home_tips -- --ignored`
    #[test]
    #[ignore]
    fn dump_home_tips_screens() {
        let Ok(dir) = std::env::var("ARGOS_SCREEN_DIR") else {
            return;
        };
        std::fs::create_dir_all(&dir).unwrap();
        let mut app = app();
        for (tip, size) in [
            (0usize, (160u16, 48u16)),
            (3, (160, 48)),
            (8, (120, 40)),
            (13, (120, 40)),
            (15, (100, 32)),
        ] {
            app.recon_tip = tip;
            dump_cells(
                &mut app,
                &dir,
                &format!("home-tip-{}-{}x{}", RECON_TIPS[tip].0, size.0, size.1),
                size.0,
                size.1,
            );
        }
    }

    /// Put `app` on one screen's loading state.
    fn loading_state(app: &mut App, screen: &str) {
        match screen {
            "jobs" => {
                app.select(ModuleId::Jobs.index());
                app.jobs.rows.clear();
                app.jobs.loaded = false;
                app.jobs.error = None;
            }
            "logs" => {
                app.select(ModuleId::Logs.index());
                app.logs.rows.clear();
                app.logs.loaded = false;
                app.logs.error = None;
            }
            "brain" => {
                app.select(ModuleId::Brain.index());
                app.memory_error = None;
                app.memories_loaded = false;
            }
            "profile" => {
                app.select(ModuleId::System.index());
                app.profile.snapshot = None;
                app.profile.error = None;
                app.profile.loading = true;
            }
            "atlas-map" => {
                app.select(ModuleId::Atlas.index());
                app.atlas_map_hold = true;
            }
            other => panic!("unknown screen {other}"),
        }
    }

    /// True when some `glyph label` occurrence sits centered (±1 cell) between
    /// its panel's borders, horizontally and vertically.
    fn loading_glyph_centered(buffer: &ratatui::buffer::Buffer, glyph: &str, label: &str) -> bool {
        let area = buffer.area;
        let sym = |x: u16, y: u16| buffer[(x, y)].symbol().to_string();
        let is_v = |s: &str| {
            matches!(
                s,
                "│" | "┃" | "║" | "┌" | "┐" | "└" | "┘" | "╭" | "╮" | "╰" | "╯"
            )
        };
        let is_h = |s: &str| {
            matches!(
                s,
                "─" | "━" | "═" | "┌" | "┐" | "└" | "┘" | "╭" | "╮" | "╰" | "╯"
            )
        };
        let text_w = 2 + label.chars().count() as u16 + 3;
        for y in 0..area.height {
            for x in 0..area.width {
                if sym(x, y) != glyph || x + text_w > area.width {
                    continue;
                }
                let row: String = (x..x + text_w - 3).map(|cx| sym(cx, y)).collect();
                if row != format!("{glyph} {label}") {
                    continue;
                }
                let Some(left) = (0..x).rev().find(|&cx| is_v(&sym(cx, y))) else {
                    continue;
                };
                let end = x + text_w;
                let Some(right) = (end..area.width).find(|&cx| is_v(&sym(cx, y))) else {
                    continue;
                };
                let Some(top) = (0..y).rev().find(|&cy| is_h(&sym(x, cy))) else {
                    continue;
                };
                let Some(bottom) = (y + 1..area.height).find(|&cy| is_h(&sym(x, cy))) else {
                    continue;
                };
                let (lg, rg) = (x - left - 1, right - end);
                let (tg, bg) = (y - top - 1, bottom - y - 1);
                if lg.abs_diff(rg) <= 1 && tg.abs_diff(bg) <= 1 {
                    return true;
                }
            }
        }
        false
    }

    const LOADING_SCREENS: [(&str, &str); 5] = [
        ("jobs", "Loading jobs"),
        ("logs", "Loading events"),
        ("brain", "Loading memories"),
        ("profile", "Loading statistics"),
        ("atlas-map", "Loading country"),
    ];

    /// Every loading state paints the shared spinner, and it advances per frame.
    #[test]
    fn loading_states_animate_across_frames() {
        for (screen, label) in LOADING_SCREENS {
            let mut frames = Vec::new();
            for index in 0..3u64 {
                let mut app = app();
                loading_state(&mut app, screen);
                super::super::anim::take_animated();
                let text = super::super::anim::with_frame(index, || {
                    buffer_text(&render(&mut app, 120, 36))
                });
                assert!(
                    super::super::anim::take_animated(),
                    "{screen}: draw marks the frame animated"
                );
                let glyph = super::super::anim::spinner_at(index);
                assert!(
                    text.contains(&format!("{glyph} {label}")),
                    "{screen} frame {index} missing `{glyph} {label}`:\n{text}"
                );
                let buffer = super::super::anim::with_frame(index, || render(&mut app, 120, 36));
                assert!(
                    loading_glyph_centered(&buffer, glyph, label),
                    "{screen} frame {index}: `{glyph} {label}` not centered in its panel:\n{text}"
                );
                frames.push(text);
            }
            assert_ne!(frames[0], frames[1], "{screen}: frame 0→1 advances");
            assert_ne!(frames[1], frames[2], "{screen}: frame 1→2 advances");
        }
    }

    /// Cell dumps of each loading state at three consecutive frames.
    /// `ARGOS_SCREEN_DIR=/workspace/screens/loading-anim cargo test -p argos-osint-bin dump_loading_animation_frames -- --ignored`
    #[test]
    #[ignore]
    fn dump_loading_animation_frames() {
        let Ok(dir) = std::env::var("ARGOS_SCREEN_DIR") else {
            return;
        };
        std::fs::create_dir_all(&dir).unwrap();
        for (screen, _) in LOADING_SCREENS {
            for index in 0..3u64 {
                let mut app = app();
                loading_state(&mut app, screen);
                super::super::anim::with_frame(index, || {
                    dump_cells(&mut app, &dir, &format!("{screen}-f{index}"), 120, 36)
                });
            }
        }
    }

    /// Defaults page with the Brief fast preset applied (phase 3).
    /// `python3 scripts/tui_review.py capture --test dump_role_routing_screens --ignored ...`
    #[test]
    #[ignore]
    fn dump_role_routing_screens() {
        let Ok(dir) = std::env::var("ARGOS_SCREEN_DIR") else {
            return;
        };
        std::fs::create_dir_all(&dir).unwrap();
        for (width, height) in [(160, 50), (120, 40)] {
            let mut app = app();
            let d = &mut app.settings.defaults;
            d.summarization.provider = "openrouter".into();
            d.summarization.model = "nvidia/nemotron-3-super-120b-a12b:free".into();
            d.claim_assessor.provider = "openrouter".into();
            d.claim_assessor.model = "nvidia/nemotron-3-ultra-550b-a55b:free".into();
            d.synthesis.provider = "openrouter".into();
            d.synthesis.model = "nvidia/nemotron-3-ultra-550b-a55b:free".into();
            d.apply_fast_preset();
            app.select(ModuleId::Providers.index());
            app.provider_page = ProviderPage::Defaults;
            app.defaults_role = DefaultsRole::Summarization;
            dump_cells(
                &mut app,
                &dir,
                &format!("defaults-fast-preset-{width}x{height}"),
                width,
                height,
            );
        }
    }

    /// Phase 4: per-operation override editing reuses the Provider/Model
    /// fields and Save controls; Reset restores inheritance.
    #[test]
    fn defaults_operation_override_editing_saves_resets_and_shows_evidence() {
        let mut app = app();
        let dir = tempfile::tempdir().unwrap();
        app.settings_path = dir.path().join("config.toml");
        app.settings.defaults.synthesis.provider = "openrouter".into();
        app.settings.defaults.synthesis.model = "openai/gpt-4o".into();
        app.select(ModuleId::Providers.index());
        app.provider_page = ProviderPage::Defaults;
        app.defaults_role = DefaultsRole::Summarization;
        app.summarization_provider = "openrouter".into();
        app.summarization_model = "google/gemma-4-31b-it:free".into();
        // Claims → Inferences → Inference support → Links.
        for _ in 0..3 {
            app.activate_button(ButtonId::BriefOpCycle);
        }
        assert_eq!(app.selected_brief_op().0, provider::OP_LINKS_GENERATE);
        app.activate_button(ButtonId::BriefOpSave);
        assert!(
            app.status.starts_with("Links override saved"),
            "{}",
            app.status
        );
        // Persisted, not just in memory.
        let saved = std::fs::read_to_string(&app.settings_path).unwrap();
        assert!(saved.contains("gemma-4-31b-it"), "{saved}");
        let o = app
            .settings
            .defaults
            .operation_overrides
            .get(provider::OP_LINKS_GENERATE)
            .expect("override saved");
        let (a, overridden) = app
            .settings
            .defaults
            .resolve_operation(Some(provider::OP_LINKS_GENERATE), "synthesis");
        assert!(overridden, "{o:?}");
        assert_eq!(a.model, "google/gemma-4-31b-it:free");
        // Role defaults are untouched.
        assert_eq!(app.settings.defaults.synthesis.model, "openai/gpt-4o");
        let text = buffer_text(&render(&mut app, 160, 50));
        assert!(text.contains("Op: Links"), "{text}");
        assert!(
            text.contains("Source: explicit operation override"),
            "{text}"
        );
        assert!(text.contains("Contract: generated_json"), "{text}");
        assert!(text.contains("Adapter: openrouter"), "{text}");
        // The evidence line fits at 160x50 without scrolling.
        assert!(text.contains("Evidence: conservative"), "{text}");
        app.activate_button(ButtonId::BriefOpReset);
        assert!(
            app.status.starts_with("Links override reset"),
            "{}",
            app.status
        );
        assert!(app.settings.defaults.operation_overrides.is_empty());
        let text = buffer_text(&render(&mut app, 160, 50));
        assert!(text.contains("Source: inherited from synthesis"), "{text}");
    }

    /// Phase 4 screens: Defaults per-operation editor and a brief with both
    /// roots running at once.
    #[test]
    #[ignore]
    fn dump_role_routing_phase4_screens() {
        let Ok(dir) = std::env::var("ARGOS_SCREEN_DIR") else {
            return;
        };
        std::fs::create_dir_all(&dir).unwrap();
        for (width, height) in [(160, 50), (120, 40)] {
            let mut app = app();
            let d = &mut app.settings.defaults;
            d.synthesis.provider = "openrouter".into();
            d.synthesis.model = "nvidia/nemotron-3-ultra-550b-a55b:free".into();
            d.claim_assessor.provider = "openrouter".into();
            d.claim_assessor.model = "nvidia/nemotron-3-ultra-550b-a55b:free".into();
            d.operation_overrides.insert(
                provider::OP_LINKS_GENERATE.into(),
                provider::ModelAssignment {
                    provider: "openrouter".into(),
                    model: "google/gemma-4-31b-it:free".into(),
                    ..Default::default()
                }
                .into(),
            );
            app.select(ModuleId::Providers.index());
            app.provider_page = ProviderPage::Defaults;
            app.defaults_role = DefaultsRole::Synthesis;
            app.brief_op_sel = 3;
            dump_cells(
                &mut app,
                &dir,
                &format!("phase4-defaults-operation-override-{width}x{height}"),
                width,
                height,
            );
        }
    }

    #[test]
    #[ignore]
    fn dump_brain_graph_screens() {
        let Ok(dir) = std::env::var("ARGOS_SCREEN_DIR") else {
            return;
        };
        std::fs::create_dir_all(&dir).unwrap();
        for (width, height) in [(160, 50), (120, 40)] {
            let mut app = app();
            app.settings.defaults.synthesis.provider = "openrouter".into();
            app.settings.defaults.synthesis.model = "openai/gpt-4o".into();
            app.settings.defaults.operation_overrides.insert(
                argos_osint_core::provider::OP_LINKS_GENERATE.into(),
                argos_osint_core::provider::ModelAssignment {
                    provider: "google".into(),
                    model: "gemini-2.5-flash".into(),
                    ..Default::default()
                }
                .into(),
            );
            app.select(ModuleId::Providers.index());
            app.provider_page = ProviderPage::Defaults;
            app.defaults_role = DefaultsRole::Synthesis;
            dump_cells(
                &mut app,
                &dir,
                &format!("models-defaults-{width}x{height}"),
                width,
                height,
            );
        }
    }

    /// Two retained cycles indexed through Atlas phase 6: actors, mention
    /// Links, Related leads, plus one evidence memory. Returns the actor ids
    /// of "Ana Ruiz" in each cycle (merged into one canonical actor).
    fn brain_world(app: &App) -> (String, String) {
        use argos_osint_core::atlas_metadata::{ArticleMention, ArticleMetadata, MetadataState};
        let store = &app.store;
        let rows = [
            (
                "run-1012",
                "k1",
                "Ana Ruiz opens Gdansk port talks",
                "2026-10-12T08:00:00Z",
                vec!["Ana Ruiz"],
            ),
            (
                "run-1012",
                "k2",
                "Ana Ruiz meets Northwind Ferries board",
                "2026-10-12T09:30:00Z",
                vec!["Ana Ruiz", "Northwind Ferries"],
            ),
            (
                "run-1013",
                "k3",
                "Northwind Ferries resumes Baltic crossings",
                "2026-10-13T06:10:00Z",
                vec!["Northwind Ferries"],
            ),
            (
                "run-1013",
                "k2",
                "Ana Ruiz meets Northwind Ferries board",
                "2026-10-12T09:30:00Z",
                vec!["Ana Ruiz", "Northwind Ferries"],
            ),
            (
                "run-1013",
                "k4",
                "Ana Ruiz signs Gdansk ferry accord",
                "2026-10-13T07:45:00Z",
                vec!["Ana Ruiz"],
            ),
        ];
        for (run, id, title, at, names) in rows {
            let title = format!("{title} in Baltic");
            let _ =
                store.atlas_insert_run(run, "{}", r#"{"contract_version":"atlas-collection-v2"}"#);
            let article = AtlasArticleRow {
                run_id: run.into(),
                id: id.into(),
                title: title.clone(),
                url: format!("https://news.example/{id}"),
                country: "PL".into(),
                source_name: "Baltic Wire".into(),
                source_domain: "news.example".into(),
                published_at: at.into(),
                provider: "gnews".into(),
                temperature: 0.5,
                category: "economic".into(),
                seen_at: at.into(),
                ..Default::default()
            };
            store.atlas_upsert_article(&article).unwrap();
            let mut m = ArticleMetadata::pending(&article);
            m.entity_state = MetadataState::Complete;
            for n in names.into_iter().chain(std::iter::once("Baltic")) {
                let start = title.find(n).unwrap();
                m.mentions.push(ArticleMention {
                    type_hint: if n == "Baltic" {
                        "place"
                    } else if n.contains("Ferries") {
                        "organization"
                    } else {
                        "person"
                    }
                    .into(),
                    comparison_key: n.to_lowercase(),
                    field: "title".into(),
                    start_byte: start,
                    end_byte: start + n.len(),
                    original_text: n.into(),
                });
            }
            store.atlas_save_metadata(&m).unwrap();
        }
        for run in ["run-1012", "run-1013"] {
            argos_osint_core::related_articles::build(store, run).unwrap();
            let report =
                argos_osint_core::atlas_actor_index::index_run(store, run, &|| false).unwrap();
            assert!(report.complete(), "{report:?}");
        }
        let actor = |article: &str| -> String {
            store
                .connection()
                .query_row(
                    "SELECT m.actor_id FROM brain_actor_mentions m JOIN brain_actors a ON a.actor_id=m.actor_id
                     WHERE m.origin_id=?1 AND a.display_name='Ana Ruiz' LIMIT 1",
                    [article],
                    |r| r.get(0),
                )
                .unwrap()
        };
        let (k1, k4) = (actor("k1"), actor("k4"));
        store
            .merge_actors(
                &k4,
                &k1,
                &serde_json::json!({"analyst":"fixture","reason":"same official"}),
            )
            .unwrap();
        store
            .add_memory(
                "Ana Ruiz signed the Gdansk ferry accord",
                "fact",
                false,
                MemorySource {
                    app: "atlas".into(),
                    conversation_id: "run-1013".into(),
                    message_id: None,
                    reference: None,
                },
            )
            .unwrap();
        (k1, k4)
    }

    fn open_kind(app: &mut App, kind: argos_osint_core::memory_metadata::MemoryKind) {
        app.select(ModuleId::Brain.index());
        app.brain_kind = Some(kind);
        app.reload_memories();
    }

    #[test]
    fn brain_kind_filter_cycles_claims_actors_with_unique_names_and_read_only_graphs() {
        use argos_osint_core::memory_metadata::MemoryKind;
        let mut app = app();
        brain_world(&app);
        app.select(ModuleId::Brain.index());
        let groups = app.store.actor_groups().unwrap();
        assert_eq!(app.memories.len(), groups.len());
        let mut seen = Vec::new();
        for _ in 0..3 {
            app.cycle_brain_kind();
            seen.push(app.brain_kind);
        }
        assert_eq!(
            seen,
            vec![Some(MemoryKind::Claim), Some(MemoryKind::Actor), None]
        );
        let key = groups
            .iter()
            .find(|g| g.display_name == "Ana Ruiz")
            .unwrap()
            .key();
        assert!(app.open_actor_group(&key));
        assert_eq!(app.brain_detail.related.tab, RelatedTab::Claims);
        app.select_brain_related_tab(2);
        assert_eq!(app.brain_detail.related.items.len(), 3);
        let text = buffer_text(&render(&mut app, 160, 50));
        assert!(
            text.contains("Entity") && text.contains("Articles"),
            "{text}"
        );
        let order = super::super::ui::focus_order(&app);
        assert!(order.contains(&Target::DetailPath));
        assert!(!app.open_memory_detail(&groups[0].actor_ids[0]));
        assert_eq!(app.brain_detail.memory_id(), Some(key.as_str()));
    }

    #[test]
    fn actor_associations_deduplicate_urls_across_retained_cycles() {
        let mut app = app();
        brain_world(&app);
        let group = app
            .store
            .actor_groups()
            .unwrap()
            .into_iter()
            .find(|g| g.display_name == "Ana Ruiz")
            .unwrap();
        let a = app.store.actor_group_associations(&group).unwrap();
        assert_eq!(a.articles.len(), 3);
        assert_eq!(
            a.articles
                .iter()
                .map(|a| a.occurrences.len())
                .sum::<usize>(),
            4
        );
        app.open_actor_group(&group.key());
        app.select_brain_related_tab(2);
        let all = app.brain_detail.related.items.len();
        app.toggle_related_scope();
        assert_eq!(
            app.brain_detail.related.items.len(),
            all,
            "Brain is always all retained origins"
        );
    }

    #[test]
    fn brain_related_tabs_keep_independent_selection_and_scroll() {
        let mut app = app();
        brain_world(&app);
        let group = app
            .store
            .actor_groups()
            .unwrap()
            .into_iter()
            .find(|g| g.display_name == "Ana Ruiz")
            .unwrap();
        assert!(app.open_actor_group(&group.key()));
        app.select_brain_related_tab(2);
        app.brain_detail.related.sel = 2;
        app.brain_detail.related.scroll = 3;
        app.select_brain_related_tab(0);
        assert_eq!(app.brain_detail.related.sel, 0);
        app.select_brain_related_tab(2);
        assert_eq!(app.brain_detail.related.sel, 2);
        assert_eq!(app.brain_detail.related.scroll, 3);
    }

    #[test]
    #[ignore]
    fn dump_brain_phase5_screens() {
        use argos_osint_core::memory_metadata::MemoryKind;
        let Ok(dir) = std::env::var("ARGOS_SCREEN_DIR") else {
            return;
        };
        std::fs::create_dir_all(&dir).unwrap();
        for (width, height) in [(160, 50), (120, 40)] {
            let mut app = app();
            brain_world(&app);
            open_kind(&mut app, MemoryKind::Related);
            let lead = app.memories[0].id.clone();
            assert!(app.open_memory_detail(&lead));
            app.brain_detail.related.tab = RelatedTab::Articles;
            dump_cells(
                &mut app,
                &dir,
                &format!("related-scope-all-{width}x{height}"),
                width,
                height,
            );
            app.toggle_related_scope();
            dump_cells(
                &mut app,
                &dir,
                &format!("related-scope-cycle-{width}x{height}"),
                width,
                height,
            );
            let mut app = self::app();
            brain_world(&app);
            let stats = serde_json::to_value(argos_osint_core::atlas::RunStats {
                contract_version: argos_osint_core::atlas_work::COLLECTION_CONTRACT.into(),
                coverage: argos_osint_core::atlas::CoverageStats {
                    articles: 3,
                    classified: 3,
                    entities_available: 3,
                    publishers_available: 3,
                    related_pairs: 2,
                    actor_indexed: 2,
                    actor_index_failed: 1,
                    ..Default::default()
                },
                ..Default::default()
            })
            .unwrap();
            app.store
                .atlas_insert_run(
                    "run-1014",
                    r#"{"phase":6,"chunk":0,"leg":"actor_articles","country":0,"from":""}"#,
                    &stats.to_string(),
                )
                .unwrap();
            app.store
                .atlas_set_state(
                    "run-1014",
                    "partial",
                    "Classification 3/3 · legacy unk 0 · entities 3 · publisher priors 3 · Related 2 · actors indexed 2/3 (zero-actor 0, failed 1) · acquisition failures 0",
                    true,
                )
                .unwrap();
            app.select(ModuleId::Atlas.index());
            if let Some(i) = app.atlas_runs.iter().position(|r| r.id == "run-1014") {
                app.atlas_run_sel = i;
            }
            dump_cells(
                &mut app,
                &dir,
                &format!("atlas-phase6-partial-{width}x{height}"),
                width,
                height,
            );
        }
    }

    #[test]
    #[ignore]
    fn dump_brain_kind_screens() {
        use argos_osint_core::memory_metadata::MemoryKind;
        let Ok(dir) = std::env::var("ARGOS_SCREEN_DIR") else {
            return;
        };
        std::fs::create_dir_all(&dir).unwrap();
        for (width, height) in [
            (160, 50),
            (120, 40),
            (100, 32),
            (80, 24),
            (60, 18),
            (40, 20),
        ] {
            let mut app = app();
            brain_world(&app);
            let group = app.store.actor_group("ana ruiz").unwrap().unwrap();
            let thread = app
                .store
                .new_thread("Ana Ruiz maritime investigation")
                .unwrap();
            app.store.set_recall_insights(&thread.id, true).unwrap();
            app.store
                .record_thread_actor(&thread.id, "Ana Ruiz", "person")
                .unwrap();
            for n in 0..42 {
                let id = format!("overflow-{n}");
                app.store.atlas_upsert_article(&AtlasArticleRow {run_id:"run-1013".into(),id:id.clone(),title:format!("Ana Ruiz and Northwind Ferries discuss a long cross-border maritime transport agreement in Baltic {n}"),url:format!("https://news.example/overflow-{n}"),..Default::default()}).unwrap();
                let article = app.store.atlas_article("run-1013", &id).unwrap().unwrap();
                let mut metadata =
                    argos_osint_core::atlas_metadata::ArticleMetadata::pending(&article);
                metadata.entity_state = argos_osint_core::atlas_metadata::MetadataState::Complete;
                for (name, kind) in [("Ana Ruiz", "person"), ("Baltic", "place")] {
                    let start = article.title.find(name).unwrap();
                    metadata
                        .mentions
                        .push(argos_osint_core::atlas_metadata::ArticleMention {
                            type_hint: kind.into(),
                            comparison_key: name.to_lowercase(),
                            field: "title".into(),
                            start_byte: start,
                            end_byte: start + name.len(),
                            original_text: name.into(),
                        });
                }
                app.store.atlas_save_metadata(&metadata).unwrap();
                argos_osint_core::brain_actors::record_mention(
                    app.store.connection(),
                    &group.actor_ids[0],
                    "article",
                    "run-1013",
                    &id,
                    "",
                    &serde_json::json!([]),
                    "fixture",
                )
                .unwrap();
            }
            argos_osint_core::related_articles::build(&app.store, "run-1013").unwrap();
            assert!(
                app.store
                    .actor_group_associations(&group)
                    .unwrap()
                    .articles
                    .len()
                    > 40,
                "capture must exercise association overflow"
            );
            let ids = claim_fixture(&app);
            app.select(ModuleId::Brain.index());
            dump_cells(
                &mut app,
                &dir,
                &format!("brain-list-all-{width}x{height}"),
                width,
                height,
            );
            app.overlay = Overlay::Help;
            dump_cells(
                &mut app,
                &dir,
                &format!("brain-help-{width}x{height}"),
                width,
                height,
            );
            app.overlay = Overlay::None;
            for kind in [MemoryKind::Claim, MemoryKind::Actor] {
                open_kind(&mut app, kind);
                dump_cells(
                    &mut app,
                    &dir,
                    &format!("brain-list-{}-{width}x{height}", kind.as_str()),
                    width,
                    height,
                );
            }
            let key = app
                .store
                .actor_groups()
                .unwrap()
                .into_iter()
                .find(|g| g.display_name == "Ana Ruiz")
                .unwrap()
                .key();
            assert!(app.open_actor_group(&key));
            for tab in 0..3 {
                app.select_brain_related_tab(tab);
                app.compact_pages[ModuleId::Brain.index()] = 2;
                app.set_focus(Target::RelatedRow(0));
                dump_cells(
                    &mut app,
                    &dir,
                    &format!("brain-actor-tab{tab}-{width}x{height}"),
                    width,
                    height,
                );
            }
            app.compact_pages[ModuleId::Brain.index()] = 0;
            app.set_focus(Target::DetailPath);
            dump_cells(
                &mut app,
                &dir,
                &format!("brain-actor-graph-{width}x{height}"),
                width,
                height,
            );
            assert!(app.open_memory_detail(&ids[0]));
            app.graph_summary="Northwind Ferries halted crossings after an engine fire. This attributed claim retains its article sources and entity references.".into();
            app.summary_failure = None;
            dump_cells(
                &mut app,
                &dir,
                &format!("brain-claim-split-{width}x{height}"),
                width,
                height,
            );
            for tab in 0..2 {
                app.select_brain_related_tab(tab);
                app.compact_pages[ModuleId::Brain.index()] = 2;
                app.set_focus(Target::RelatedRow(0));
                dump_cells(
                    &mut app,
                    &dir,
                    &format!("brain-claim-tab{tab}-{width}x{height}"),
                    width,
                    height,
                );
            }
            for (state, label) in [
                (super::super::brain_detail::RelatedState::Loading, "loading"),
                (
                    super::super::brain_detail::RelatedState::Failed("retry".into()),
                    "error",
                ),
                (super::super::brain_detail::RelatedState::Ready, "empty"),
            ] {
                app.brain_detail.related.state = state;
                app.brain_detail.related.items.clear();
                dump_cells(
                    &mut app,
                    &dir,
                    &format!("brain-associations-{label}-{width}x{height}"),
                    width,
                    height,
                );
            }
        }
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

    #[test]
    fn brain_background_delivery_preserves_input_selection_and_last_good_data() {
        let mut app = app();
        for text in ["alpha one", "beta two", "alpha three"] {
            app.store
                .add_user_claim(
                    text,
                    "fact",
                    MemorySource {
                        app: "user".into(),
                        conversation_id: "fixture".into(),
                        message_id: None,
                        reference: None,
                    },
                )
                .unwrap();
        }
        app.brain_cache = app.store.brain_browse_memories(None, "").unwrap();
        app.background_db = Some(PathBuf::from("controlled-worker"));
        app.module = Some(ModuleId::Atlas);
        app.reload_memories();
        assert!(
            app.brain_dirty && app.brain_reading.is_none(),
            "hidden Brain only invalidates its cache"
        );
        app.module = Some(ModuleId::Brain);
        app.filter_memory_cache();
        let selected = app.memories[0].id.clone();
        let old_generation = app.brain_generation;
        app.brain_reading = Some(old_generation);
        for _ in 0..20 {
            app.reload_memories();
        }
        assert_eq!(
            app.brain_reading,
            Some(old_generation),
            "at most one pending read"
        );
        app.set_focus(Target::Field(FieldId::BrainQuery));
        type_text(&mut app, "alpha");
        assert_eq!(app.brain_query, "alpha");
        assert_eq!(
            app.memories.len(),
            2,
            "Find uses cached rows during a delayed read"
        );
        app.module = Some(ModuleId::Atlas);
        let cache = app.brain_cache.clone();
        app.on_brain_loaded(old_generation, Ok(Vec::new()));
        assert_eq!(
            app.brain_cache, cache,
            "stale completion cannot clear a newer snapshot"
        );
        assert!(app.brain_reading.is_none() && app.brain_dirty);
        app.module = Some(ModuleId::Brain);
        app.brain_reading = Some(app.brain_generation);
        app.brain_query.clear();
        app.brain_prefer = Some(selected.clone());
        app.on_brain_loaded(app.brain_generation, Ok(cache));
        assert_eq!(app.memories[app.memory_sel].id, selected);
        let shown = app.memories.clone();
        app.brain_generation += 1;
        app.brain_reading = Some(app.brain_generation);
        app.on_brain_loaded(app.brain_generation, Err("database is busy".into()));
        assert_eq!(app.memories, shown);
        assert_eq!(app.memory_error.as_deref(), Some("database is busy"));
    }

    #[test]
    fn background_event_bursts_leave_time_for_navigation() {
        let mut app = app();
        for _ in 0..100 {
            app.work_tx
                .send(WorkEvent::BrainLoaded {
                    generation: 0,
                    outcome: Ok(Vec::new()),
                })
                .unwrap();
        }
        assert!(pump(&mut app));
        assert!(
            app.work_rx.len() >= 68,
            "a frame handles at most 32 work events"
        );
        app.handle_key(KeyEvent::new(KeyCode::Char('3'), KeyModifiers::NONE));
        assert_eq!(app.module, Some(ModuleId::Brain));
        assert!(
            !app.work_rx.is_empty(),
            "navigation runs while a burst is still queued"
        );
    }

    #[test]
    fn recalled_memory_popup_uses_the_recon_snapshot_after_database_loss() {
        let mut app = app();
        let memory = app
            .store
            .add_user_claim(
                "Northwind closed the port",
                "fact",
                MemorySource {
                    app: "user".into(),
                    conversation_id: "fixture".into(),
                    message_id: None,
                    reference: None,
                },
            )
            .unwrap();
        app.store.connection().execute(
            "INSERT INTO insight_claims(fingerprint,memory_id,entity_id,predicate,object_value,topic,classification,confidence,created_at,updated_at) VALUES ('popup',?1,'Northwind','closed','the port','transport','fact',0.8,'','')",
            [&memory.id],
        ).unwrap();
        let expected = app.insight_summary(&memory.id).unwrap();
        let thread = app.store.new_thread("Port investigation").unwrap();
        let run = app
            .store
            .new_run(&thread.id, "turn", "fixture", "fixture")
            .unwrap();
        let answer = app
            .store
            .add_answer(
                &thread.id,
                &run.id,
                "Port findings",
                &[],
                std::slice::from_ref(&memory.id),
            )
            .unwrap();
        let snapshot = super::super::db_reads::ReconSnapshot::read(&app.store, &thread.id).unwrap();
        app.answer_memories = snapshot.memories;
        app.answer_memory_summaries = snapshot.memory_summaries;
        app.background_db = Some(PathBuf::from("unavailable-database"));
        app.store = Store::memory().unwrap();
        assert_eq!(
            app.insight_summary(&memory.id).as_deref(),
            Some(expected.as_str())
        );
        app.module = Some(ModuleId::Recon);
        app.overlay = Overlay::Memories {
            message_id: answer.id,
        };
        let text = buffer_text(&render(&mut app, 160, 50));
        assert!(text.contains("Northwind · closed → the port"), "{text}");
        assert!(text.contains("0 evidence links"), "{text}");
    }

    #[test]
    fn background_log_cleanup_keeps_navigation_and_failed_view() {
        let mut app = app();
        app.select(ModuleId::Logs.index());
        app.logs.open.insert("retained-event".into());
        app.logs.scroll = 3;
        app.logs.follow = false;
        app.background_db = Some(PathBuf::from("controlled-worker"));
        app.logs_clearing = true;
        app.activate_button(ButtonId::ClearLog);
        assert!(app.status.contains("already in progress"));
        app.set_focus(Target::Button(ButtonId::LogsFollow));
        app.handle_key(KeyEvent::new(KeyCode::Char('3'), KeyModifiers::NONE));
        assert_eq!(app.module, Some(ModuleId::Brain));
        app.on_work_event(WorkEvent::LogsCleared {
            outcome: Err("database busy".into()),
        });
        assert!(!app.logs_clearing);
        assert!(app.logs.open.contains("retained-event"));
        assert_eq!(app.logs.scroll, 3);
        assert!(!app.logs.follow);
        assert!(app.status.contains("Could not clear events"));
        assert_eq!(app.module, Some(ModuleId::Brain));
    }

    #[test]
    fn background_job_destination_selects_the_requested_record() {
        let mut app = app();
        register(&app, "job-one", "recon", "First investigation");
        register(&app, "job-two", "atlas", "Second cycle");
        app.select(ModuleId::Jobs.index());
        let first = app.jobs.selected().unwrap().id.clone();
        let destination = app
            .jobs
            .rows
            .iter()
            .find(|j| j.id != first)
            .unwrap()
            .id
            .clone();
        let mut snapshot = app.jobs.clone();
        assert!(snapshot.focus_job(&app.store, &destination));
        app.background_db = Some(PathBuf::from("controlled-worker"));
        app.pending_job = Some(destination.clone());
        app.dashboard_reading = true;
        let key = app.dashboard_read_key();
        app.on_work_event(WorkEvent::DashboardLoaded {
            module: Some(ModuleId::Jobs),
            key,
            outcome: Ok(Box::new(DashboardSnapshot::Jobs(Box::new(snapshot)))),
        });
        assert_eq!(app.jobs.selected().unwrap().id, destination);
        assert_eq!(app.focus, Target::JobRow(app.jobs.sel));
        assert!(app.pending_job.is_none());
    }

    #[test]
    fn brain_async_detail_back_and_escape_drop_late_navigation() {
        let mut app = app();
        brain_world(&app);
        let groups = app.store.actor_groups().unwrap();
        let first = groups[0].key();
        let second = groups[1].key();
        let a = load_brain_detail(&app.store, &first).unwrap();
        let b = load_brain_detail(&app.store, &second).unwrap();
        app.module = Some(ModuleId::Brain);
        app.background_db = Some(PathBuf::from("controlled-worker"));
        app.brain_detail_reading = true;
        app.brain_detail_target = Some(first.clone());
        app.on_brain_detail_loaded(0, Ok(Box::new(a)));
        assert_eq!(app.brain_detail.memory_id(), Some(first.as_str()));
        app.scrolls.path = 2;
        app.brain_detail_reading = true;
        app.brain_detail_target = Some(second.clone());
        app.refresh_open_detail();
        assert_eq!(
            app.brain_detail_target.as_deref(),
            Some(second.as_str()),
            "refresh preserves pending navigation"
        );
        app.on_brain_detail_loaded(0, Ok(Box::new(b)));
        assert_eq!(app.brain_detail.history.len(), 1);
        let snapshot = app.brain_detail.history.last().unwrap().clone();
        app.brain_detail_restore = Some(snapshot.clone());
        app.brain_detail_back = true;
        app.brain_detail_reading = true;
        app.brain_detail_target = Some(first.clone());
        let a = load_brain_detail(&app.store, &first).unwrap();
        app.on_brain_detail_loaded(0, Ok(Box::new(a)));
        assert!(app.brain_detail.history.is_empty());
        assert_eq!(app.scrolls.path, snapshot.path_scroll);
        let a = load_brain_detail(&app.store, &second).unwrap();
        app.brain_detail_reading = true;
        app.brain_detail_target = Some(second);
        app.leave_brain_detail();
        app.on_brain_detail_loaded(0, Ok(Box::new(a)));
        assert_eq!(app.brain_list_mode, BrainListMode::List);
    }

    #[test]
    fn dump_db_responsiveness_screens() {
        let Some(dir) = std::env::var("ARGOS_SCREEN_DIR").ok() else {
            return;
        };
        std::fs::create_dir_all(&dir).unwrap();
        for (width, height) in [
            (160, 50),
            (120, 40),
            (100, 32),
            (80, 24),
            (60, 18),
            (40, 20),
        ] {
            let mut app = app();
            app.module = Some(ModuleId::Brain);
            app.memories_loaded = false;
            app.background_db = Some(PathBuf::from("controlled-worker"));
            app.brain_reading = Some(0);
            dump_cells(
                &mut app,
                &dir,
                &format!("brain-loading-{width}x{height}"),
                width,
                height,
            );
            for text in [
                "Northwind Ferries suspended the Baltic crossing",
                "The council requested a safety assessment",
            ] {
                app.store
                    .add_user_claim(
                        text,
                        "fact",
                        MemorySource {
                            app: "user".into(),
                            conversation_id: "fixture".into(),
                            message_id: None,
                            reference: None,
                        },
                    )
                    .unwrap();
            }
            let rows = app.store.brain_browse_memories(None, "").unwrap();
            app.on_brain_loaded(0, Ok(rows));
            app.brain_reading = Some(1);
            app.focus = Target::Memory(0);
            dump_cells(
                &mut app,
                &dir,
                &format!("brain-refreshing-{width}x{height}"),
                width,
                height,
            );
            app.brain_reading = None;
            app.memory_error = Some("Database busy; retry the refresh".into());
            dump_cells(
                &mut app,
                &dir,
                &format!("brain-read-error-{width}x{height}"),
                width,
                height,
            );
            app.module = Some(ModuleId::Intel);
            app.intel_page = IntelPage::Bulletin;
            app.intel_list_reading = true;
            app.intel_articles.clear();
            dump_cells(
                &mut app,
                &dir,
                &format!("intel-bulletin-loading-{width}x{height}"),
                width,
                height,
            );
            app.module = Some(ModuleId::Atlas);
            app.atlas_page = AtlasPage::Runs;
            app.atlas_news = true;
            app.atlas_news_run = "fixture".into();
            app.atlas_articles_reading = true;
            app.atlas_articles.clear();
            app.focus = Target::Button(ButtonId::AtlasWorld);
            dump_cells(
                &mut app,
                &dir,
                &format!("atlas-news-loading-{width}x{height}"),
                width,
                height,
            );
        }
    }

    #[test]
    fn insight_reliability_screens() {
        use argos_osint_core::{
            investigation::insight_execution::Progress,
            jobs_view::{JobDetail, JobRow, TaskRow},
        };
        let dir = std::env::var("ARGOS_SCREEN_DIR").ok();
        if let Some(dir) = &dir {
            std::fs::create_dir_all(dir).unwrap();
        }
        for (width, height) in [(160, 50), (80, 24), (60, 18)] {
            let mut app = app();
            let (_, article) = related_ui_fixture(&mut app);
            for state in ["failed", "published"] {
                let mut progress=Progress {artifacts:Vec::new(),task_id:"insight-fixture-task".into(),job_id:"insight-fixture-job".into(),retry_epoch:1,stage:if state=="failed" {"assessment"} else {"published"}.into(),state:if state=="failed" {"failed"} else {"completed"}.into(),windows_done:2,windows_total:2,batches_done:1,batches_total:2,withheld_count:3,route:"Route: local/fixture · attempt 2 · fail".into(),error:if state=="failed" {"assessment: invalid_result; missing question q1:publication after bounded repair"} else {"optional link batch unavailable; qualified claims published"}.into(),retryable:true,index_status:"Index pending: 5/5".into(),activity:String::new(),activity_age_secs:None};
                progress.artifacts =
                    argos_osint_core::investigation::insight_execution::artifact_steps(
                        &progress,
                        &Default::default(),
                    );
                app.select(ModuleId::Intel.index());
                app.intel_articles = vec![article.clone()];
                app.intel_sel = 0;
                app.intel_page = IntelPage::Briefing;
                app.intel_insight_lifecycle = if state == "failed" {
                    "previous"
                } else {
                    "active"
                }
                .into();
                app.intel_extraction_progress = Some(progress.clone());
                app.compact_pages[ModuleId::Intel.index()] = 0;
                app.scrolls.intel_extracted = 0;
                let buffer = render(&mut app, width, height);
                assert!(
                    super::super::ui::focus_order(&app).contains(&Target::IntelPreview)
                        || width < 100
                );
                if width == 160 {
                    // Progression details live in Background jobs, not panes.
                    let text = buffer_text(&buffer);
                    assert!(!text.contains("Withheld"));
                    assert!(text.contains("Claims · "), "{text}");
                }
                if let Some(dir) = &dir {
                    dump_cells(
                        &mut app,
                        dir,
                        &format!("insight-intel-{state}-{width}x{height}"),
                        width,
                        height,
                    );
                }
                let job = JobRow {
                    id: progress.job_id.clone(),
                    app: "intel".into(),
                    operation: "insight_refresh".into(),
                    title: "Assess and refresh investigative insights".into(),
                    state: progress.state.clone(),
                    ..Default::default()
                };
                app.select(ModuleId::Jobs.index());
                app.jobs.rows = vec![job.clone()];
                app.jobs.sel = 0;
                app.jobs.detail = Some(JobDetail {
                    job,
                    retryable: 1,
                    tasks: vec![TaskRow {
                        id: progress.task_id.clone(),
                        operation: "insight_refresh".into(),
                        state: progress.state.clone(),
                        ..Default::default()
                    }],
                    insight_progress: vec![progress],
                    ..Default::default()
                });
                app.jobs.detail_open = true;
                app.jobs.detail_scroll = 0;
                let buffer = render(&mut app, width, height);
                assert!(buffer_text(&buffer).contains("Retry extraction"));
                let target = Target::Button(ButtonId::JobsRetry);
                assert!(super::super::ui::focus_order(&app).contains(&target));
                assert!(hit(&app, target));
                if let Some(dir) = &dir {
                    dump_cells(
                        &mut app,
                        dir,
                        &format!("insight-jobs-{state}-{width}x{height}"),
                        width,
                        height,
                    );
                }
            }
            app.select(ModuleId::Intel.index());
            app.overlay = Overlay::None;
            let items = app.palette_items();
            assert!(items.iter().any(|i| i.id == "intel-extraction-retry"));
            assert!(items.iter().any(|i| i.id == "intel-extraction-job"));
            app.open_palette();
            app.palette_query = "extraction".into();
            let buffer = render(&mut app, width, height);
            assert!(buffer_text(&buffer).contains("Retry extraction"));
            if let Some(dir) = &dir {
                dump_cells(
                    &mut app,
                    dir,
                    &format!("insight-palette-{width}x{height}"),
                    width,
                    height,
                );
            }
        }
    }

    /// Render text of every compact Intel page (narrow layouts page the
    /// Extracted / Reader / Context columns); a single page when wide.
    fn intel_all_pages_text(app: &mut App, width: u16, height: u16) -> String {
        let mut text = String::new();
        for page in 0..3 {
            app.compact_pages[ModuleId::Intel.index()] = page;
            text.push_str(&buffer_text(&render(app, width, height)));
        }
        app.compact_pages[ModuleId::Intel.index()] = 0;
        text
    }

    /// Spec §6/§8.8: shared stage progress for the v2 brief graph (two
    /// simultaneous roots, then the join) on
    /// the brief Jobs panel, section headers and global Jobs. Dumps cells
    /// when ARGOS_SCREEN_DIR is set.
    #[test]
    fn intel_sequential_stage_screens() {
        use argos_osint_core::intel_recon::brief_pipeline as bp;
        use argos_osint_core::tasks;
        let dir = std::env::var("ARGOS_SCREEN_DIR").ok();
        if let Some(dir) = &dir {
            std::fs::create_dir_all(dir).unwrap();
        }
        let now = || chrono::Utc::now().to_rfc3339();
        let body = serde_json::json!({"markdown":"Officials said the crossing would stay closed.","content_hash":"h","quality":"complete","diagnostics":[]});
        let claims = serde_json::json!({"claims":[{"id":"c1","text":"Baltic board suspended the ferry service.","attributed_to":"Baltic board","modality":"asserted","evidence_ids":["t1"],"fidelity":"faithful"}],"withheld":0});
        let inferences = serde_json::json!({"inferences":[{"id":"i1","text":"The suspension may reflect an insurance lapse, not only weather.","support":"conditional","why":"The harbour master confirmed the certificate expired.","limitations":["Whether the lapse stopped sailings is unconfirmed"],"body_evidence_ids":["b1"]}]});
        let empty_inf =
            serde_json::json!({"inferences":[],"empty_reason":"no supported inferences"});
        // (name, steps completed, running activity, fail stage, empty inferences)
        let frames: Vec<(&str, usize, Option<&str>, bool, bool)> = vec![
            ("1-queued", 0, None, false, false),
            (
                "2-roots-running",
                0,
                Some("retrieving / cleaning"),
                false,
                false,
            ),
            (
                "3-inferences-running",
                2,
                Some("waiting for model"),
                false,
                false,
            ),
            ("4-inferences-failed", 2, None, true, false),
            ("5-completed", 4, None, false, false),
            ("6-empty", 4, None, false, true),
            ("7-cached", 4, None, false, false),
        ];
        // §8.8 widths; narrow and too-small sizes render the failed and
        // completed frames (geometry changes there), wider ones every state.
        for (width, height) in [
            (160, 50),
            (120, 40),
            (100, 32),
            (80, 24),
            (60, 18),
            (40, 12),
        ] {
            for (name, done, running, fail, empty) in &frames {
                if width <= 80 && !(name.starts_with('4') || name.starts_with('5')) {
                    continue;
                }
                let mut app = app();
                let (_, article) = related_ui_fixture(&mut app);
                let lead_target = app
                    .store
                    .atlas_list_articles(&article.run_id)
                    .unwrap()
                    .into_iter()
                    .find(|a| a.id != article.id)
                    .unwrap();
                let links = serde_json::json!({"leads":[{"article_id":lead_target.id,"article_run_id":lead_target.run_id,"inference_id":"i1","why":"Its title reports the same ferry's insurance lapse; the causal link to the suspension still needs verification.","verification_needed":"Confirm the lapse date with the regulator.","metadata_evidence":[]}]});
                app.select(ModuleId::Intel.index());
                app.intel_articles = vec![article.clone()];
                app.intel_sel = 0;
                app.open_intel_briefing();
                let job = app.intel_brief_job.clone().unwrap();
                let conn = app.store.connection();
                let outputs = [
                    body.clone(),
                    claims.clone(),
                    if *empty {
                        empty_inf.clone()
                    } else {
                        inferences.clone()
                    },
                    if *empty {
                        serde_json::json!({"leads":[],"empty_reason":"no supported inferences"})
                    } else {
                        links.clone()
                    },
                ];
                for _ in 0..*done {
                    let c = bp::claim_stage(conn, "w", 120, &now()).unwrap().unwrap();
                    // Roots may be claimed in either order: commit by stage.
                    let n = bp::STAGES
                        .iter()
                        .position(|(op, _)| *op == c.operation)
                        .unwrap();
                    let out = &outputs[n];
                    let items = out
                        .as_object()
                        .and_then(|o| o.values().find_map(|v| v.as_array().map(Vec::len)))
                        .unwrap_or(0);
                    let reason = if n == 0 {
                        "validated clean body"
                    } else {
                        out["empty_reason"].as_str().unwrap_or("")
                    };
                    let items = if n == 0 { 0 } else { items };
                    assert!(
                        bp::commit_stage(conn, &c, &out.to_string(), items, reason, &now())
                            .unwrap()
                    );
                }
                if let Some(activity) = running {
                    let c = bp::claim_stage(conn, "w", 120, &now()).unwrap().unwrap();
                    bp::set_activity(conn, &c.id, activity).unwrap();
                    if *done == 0 {
                        // Both independent roots run at once.
                        let other = bp::claim_stage(conn, "w2", 120, &now()).unwrap().unwrap();
                        bp::set_activity(conn, &other.id, "waiting for model").unwrap();
                    }
                }
                if *fail {
                    let mut c = bp::claim_stage(conn, "w", 120, &now()).unwrap().unwrap();
                    c.max_attempts = 1;
                    tasks::fail_claimed(
                        conn,
                        &c,
                        tasks::OperationKind::OtherLlm,
                        &tasks::TaskError::new(
                            tasks::ErrorCategory::InvalidResult,
                            "provider chain exhausted",
                        ),
                        &now(),
                    )
                    .unwrap();
                }
                if *name == "7-cached" {
                    // Reopen a completed brief: attaches, schedules nothing.
                    app.open_intel_briefing();
                    assert_eq!(app.intel_brief_job.as_deref(), Some(job.as_str()));
                    let active: i64 = app
                        .store
                        .connection()
                        .query_row(
                            "SELECT COUNT(*) FROM argos_tasks WHERE job_id LIKE 'intel-brief:%' AND state NOT IN ('completed','superseded')",
                            [],
                            |r| r.get(0),
                        )
                        .unwrap();
                    assert_eq!(active, 0, "cached reopen enqueues no work");
                }
                app.intel_brief_stages.clear();
                app.refresh_intel_brief_stages();
                app.compact_pages[ModuleId::Intel.index()] = 0;
                app.scrolls.intel_extracted = 0;
                let text = intel_all_pages_text(&mut app, width, height);
                let rows: Vec<String> = app
                    .intel_brief_stages
                    .iter()
                    .map(|v| format!("{} · {}", v.label, v.row))
                    .collect();
                // Content asserts where the panes fit without scrolling.
                let roomy = width >= 120 || (width == 80 && name.starts_with('4'));
                match *name {
                    _ if !roomy => {
                        // Narrow / too-small: rows stay correct and render never panics.
                        assert!(!rows.is_empty());
                    }
                    "1-queued" => {
                        assert_eq!(rows[1], "Claims · Queued");
                        assert_eq!(rows[2], "Inferences · Queued · after Full article, Claims");
                        assert!(text.contains("Brief · 0/4 completed"), "{text}");
                    }
                    "2-roots-running" => {
                        assert!(rows[0].starts_with("Full article · Running"), "{rows:?}");
                        assert!(rows[1].starts_with("Claims · Running"), "{rows:?}");
                        assert!(text.contains("Brief · 2 running · 0/4 completed"), "{text}");
                    }
                    "3-inferences-running" => {
                        assert_eq!(rows[2], "Inferences · Running · waiting for model");
                        assert!(text.contains("Brief · 1 running · 2/4 completed"));
                        assert!(
                            text.contains("suspended the ferry"),
                            "claims checkpoint surfaces immediately"
                        );
                    }
                    "4-inferences-failed" => {
                        assert!(rows[1].starts_with("Claims · Completed"));
                        assert!(rows[2].starts_with("Inferences · Failed"));
                        assert_eq!(rows[3], "Links · Queued · blocked by Inferences");
                        assert!(
                            text.contains("suspended the ferry"),
                            "completed Claims kept"
                        );
                        assert!(text.contains("blocked by Inferences"));
                        // Global Jobs shows identical stage states.
                        let detail = argos_osint_core::jobs_view::job_detail(
                            app.store.connection(),
                            &job,
                            10,
                        )
                        .unwrap()
                        .unwrap();
                        let labels: Vec<&str> = detail
                            .tasks
                            .iter()
                            .map(|t| t.stage_label.as_str())
                            .collect();
                        assert_eq!(labels, rows.iter().map(String::as_str).collect::<Vec<_>>());
                    }
                    "5-completed" => {
                        assert!(text.contains("Potential corroboration"), "{text}");
                        assert!(text.contains("insurance lapse"));
                        assert!(
                            app.intel_brief_publication.starts_with("Published"),
                            "{}",
                            app.intel_brief_publication
                        );
                        assert!(text.contains("Publication / Index"));
                        assert_eq!(
                            bp::published_leads(app.store.connection(), &article.id)
                                .unwrap()
                                .len(),
                            1
                        );
                    }
                    "7-cached" => {
                        assert!(rows.iter().all(|r| r.contains("Completed")), "{rows:?}");
                        assert!(text.contains("Potential corroboration"));
                    }
                    _ => {
                        assert_eq!(rows[2], "Inferences · Completed · no supported inferences");
                        assert_eq!(rows[3], "Links · Completed · no supported inferences");
                    }
                }
                if let Some(dir) = &dir {
                    dump_cells(
                        &mut app,
                        dir,
                        &format!("seq-{name}-{width}x{height}"),
                        width,
                        height,
                    );
                    if width < 160 {
                        app.compact_pages[ModuleId::Intel.index()] = 2;
                        dump_cells(
                            &mut app,
                            dir,
                            &format!("seq-{name}-{width}x{height}-jobs"),
                            width,
                            height,
                        );
                        app.compact_pages[ModuleId::Intel.index()] = 0;
                    }
                }
            }
        }
    }

    /// Claims, Insights and Links each get their own Background jobs entry
    /// with their own streamed steps; content panes show only the shared
    /// centered loading animation until their content arrives.
    #[test]
    fn intel_artifact_progress_in_background_jobs_screens() {
        use argos_osint_core::investigation::insight_execution::{
            artifact_steps, ArtifactFacts, Progress,
        };
        let dir = std::env::var("ARGOS_SCREEN_DIR").ok();
        if let Some(dir) = &dir {
            std::fs::create_dir_all(dir).unwrap();
        }
        let synth = |assess: &[(&'static str, &str)]| ArtifactFacts {
            synthesis_started: true,
            assessments: assess.iter().map(|(a, s)| (*a, s.to_string())).collect(),
        };
        let pre = |assess: &[(&'static str, &str)]| ArtifactFacts {
            synthesis_started: false,
            assessments: assess.iter().map(|(a, s)| (*a, s.to_string())).collect(),
        };
        // (name, state, stage, activity, age, facts, expected [claims, insights, links])
        type Frame<'a> = (
            &'a str,
            &'a str,
            &'a str,
            &'a str,
            Option<u64>,
            ArtifactFacts,
            [&'a str; 3],
        );
        let frames: Vec<Frame> = vec![
            (
                "1-queued",
                "queued",
                "queued",
                "",
                None,
                pre(&[]),
                ["Queued", "Queued", "Queued"],
            ),
            (
                "2-claims-waiting",
                "running",
                "extraction",
                "Waiting for output · attempt 1",
                Some(6),
                pre(&[]),
                [
                    "Waiting for output",
                    "Queued · after claims",
                    "Queued · after claims",
                ],
            ),
            (
                "3-claims-receiving",
                "running",
                "extraction",
                "Receiving output · attempt 1 · 1840 chars",
                Some(1),
                pre(&[]),
                [
                    "Receiving output",
                    "Queued · after claims",
                    "Queued · after claims",
                ],
            ),
            (
                "4-claims-stalled",
                "running",
                "extraction",
                "Receiving output · attempt 1 · 1840 chars",
                Some(97),
                pre(&[]),
                [
                    "Stalled · no model activity for 97s",
                    "Queued · after claims",
                    "Queued · after claims",
                ],
            ),
            (
                "5-claims-validating",
                "running",
                "assessment",
                "Validating",
                Some(0),
                pre(&[("Claims", "running")]),
                [
                    "Validating",
                    "Queued · after claims",
                    "Queued · after claims",
                ],
            ),
            (
                "6-synthesis-receiving",
                "running",
                "inference",
                "Receiving output · attempt 1 · 3120 chars",
                Some(1),
                synth(&[("Claims", "completed")]),
                [
                    "Validated · awaiting publication",
                    "Receiving output",
                    "Receiving output",
                ],
            ),
            (
                "7-insights-decision",
                "running",
                "assessment",
                "",
                Some(3),
                synth(&[("Claims", "completed"), ("Insights", "running")]),
                [
                    "Validated · awaiting publication",
                    "Waiting for decision",
                    "Queued · awaiting assessment",
                ],
            ),
            (
                "8-links-stalled",
                "running",
                "assessment",
                "Waiting for decision · attempt 1",
                Some(200),
                synth(&[
                    ("Claims", "completed"),
                    ("Insights", "completed"),
                    ("Links", "running"),
                ]),
                [
                    "Validated · awaiting publication",
                    "Validated · awaiting publication",
                    "Stalled · no model activity for 200s",
                ],
            ),
            (
                "9-published",
                "completed",
                "published",
                "",
                None,
                synth(&[]),
                ["Published", "Published", "Published"],
            ),
        ];
        for (width, height) in [(160, 50), (120, 40)] {
            for (name, state, stage, activity, age, facts, expected) in &frames {
                let mut app = app();
                let (_, article) = related_ui_fixture(&mut app);
                let mut progress = Progress {
                    task_id: "insight-stream-task".into(),
                    job_id: "insight-stream-job".into(),
                    stage: (*stage).into(),
                    state: (*state).into(),
                    windows_total: 1,
                    route:
                        "Route: local/nvidia/nemotron-3-super-120b-a12b:free · attempt 1 · running"
                            .into(),
                    index_status: "Index pending: 0/0".into(),
                    activity: (*activity).into(),
                    activity_age_secs: *age,
                    ..Default::default()
                };
                progress.artifacts = artifact_steps(&progress, facts);
                let got: Vec<&str> = progress.artifacts.iter().map(|a| a.step.as_str()).collect();
                assert_eq!(got, expected.to_vec(), "{name}");
                let published = *state == "completed";
                app.select(ModuleId::Intel.index());
                app.intel_articles = vec![article.clone()];
                app.intel_sel = 0;
                app.intel_page = IntelPage::Briefing;
                app.intel_insight_lifecycle =
                    if published { "active" } else { "refreshing" }.into();
                app.intel_refresh_status = expected[0].into();
                if published {
                    let claim = |fp: &str, class: &str, text: &str| AtlasArticleClaim {
                        fingerprint: fp.into(),
                        entity: "Baltic board".into(),
                        predicate: "said".into(),
                        object: "ferry".into(),
                        topic: "investigation".into(),
                        classification: class.into(),
                        confidence: 0.0,
                        claim: text.into(),
                        source_url: article.url.clone(),
                        published_at: String::new(),
                        article_id: article.id.clone(),
                        reliability: String::new(),
                        info_credibility: 0,
                        admiralty: String::new(),
                        rsp_status: String::new(),
                    };
                    app.intel_claims = vec![
                        claim(
                            "v2:c1",
                            "claim",
                            "[claim · faithful] Baltic board suspended the ferry service.",
                        ),
                        claim(
                            "v2:i1",
                            "inference",
                            "[inference · supported] The suspension is temporary pending review.",
                        ),
                    ];
                    app.intel_relations = vec![("v2:i1".into(), "v2:c1".into(), "supports".into())];
                } else {
                    app.intel_claims.clear();
                    app.intel_relations.clear();
                    app.intel_link_explanations.clear();
                }
                app.intel_extraction_progress = Some(progress.clone());
                app.compact_pages[ModuleId::Intel.index()] = 0;
                app.scrolls.intel_extracted = 0;
                let text = intel_all_pages_text(&mut app, width, height);
                // Background jobs: each artifact has its own entry and step.
                for (artifact, step) in ["Claims", "Insights", "Links"].iter().zip(expected) {
                    let short = step.split(" · no model activity").next().unwrap();
                    let short = short.split(" · awaiting").next().unwrap();
                    let short = short.split(" · after").next().unwrap();
                    let entry = format!("{artifact} · {short}");
                    let shown = text.contains(&entry);
                    assert!(shown, "{name} {width}x{height}: missing {entry}\n{text}");
                }
                if expected.iter().any(|s| s.starts_with("Stalled")) {
                    assert!(
                        text.contains("⚠ no model activity for"),
                        "{name}: stall warning"
                    );
                }
                // Content panes never carry the streamed details.
                for leaked in ["Stage: ", "Withheld: ", "Route: "] {
                    assert!(
                        !text.contains(leaked),
                        "{name}: '{leaked}' leaked into a pane"
                    );
                }
                // Centered spinner until content arrives; content replaces it.
                for label in [
                    "Extracting claims",
                    "Extracting inferences",
                    "Extracting links",
                ] {
                    assert_eq!(
                        text.contains(label),
                        !published,
                        "{name} {width}x{height}: {label}\n{text}"
                    );
                }
                if published {
                    assert!(
                        text.contains("suspended the ferry"),
                        "{name}: claims content"
                    );
                    assert!(
                        text.contains("temporary pending"),
                        "{name}: inference content"
                    );
                }
                if let Some(dir) = &dir {
                    dump_cells(
                        &mut app,
                        dir,
                        &format!("artifacts-{name}-{width}x{height}"),
                        width,
                        height,
                    );
                    if width < 160 {
                        app.compact_pages[ModuleId::Intel.index()] = 2;
                        dump_cells(
                            &mut app,
                            dir,
                            &format!("artifacts-{name}-{width}x{height}-jobs"),
                            width,
                            height,
                        );
                        app.compact_pages[ModuleId::Intel.index()] = 0;
                    }
                }
            }
        }
    }

    /// Spec §6: streamed insight status steps on Intel (Focus Brief), Jobs
    /// and Logs. Dumps PNG-able cells when ARGOS_SCREEN_DIR is set.
    #[test]
    fn streamed_insight_progress_screens() {
        use argos_osint_core::{
            investigation::insight_execution::{status_step, Progress},
            jobs_view::{JobDetail, JobRow, TaskRow},
        };
        let dir = std::env::var("ARGOS_SCREEN_DIR").ok();
        if let Some(dir) = &dir {
            std::fs::create_dir_all(dir).unwrap();
        }
        // (name, task state, stage, activity, age)
        let steps: [(&str, &str, &str, &str, Option<u64>); 7] = [
            ("1-queued", "queued", "queued", "", None),
            (
                "2-waiting",
                "running",
                "extraction",
                "Waiting for output · attempt 1 · nvidia/nemotron-3-super-120b-a12b:free",
                Some(12),
            ),
            (
                "3-receiving",
                "running",
                "extraction",
                "Receiving output · attempt 1 · 2814 chars",
                Some(1),
            ),
            (
                "4-validating",
                "running",
                "extraction",
                "Validating",
                Some(0),
            ),
            (
                "5-decision",
                "running",
                "assessment",
                "Waiting for decision",
                Some(4),
            ),
            (
                "6-stalled",
                "running",
                "extraction",
                "Receiving output · attempt 2 · 912 chars",
                Some(97),
            ),
            ("7-published", "completed", "published", "", None),
        ];
        for (width, height) in [(160, 50), (120, 40)] {
            for (name, state, stage, activity, age) in steps {
                let mut app = app();
                let (_, article) = related_ui_fixture(&mut app);
                let mut progress = Progress {
                    task_id: "insight-stream-task".into(),
                    job_id: "insight-stream-job".into(),
                    retry_epoch: 0,
                    stage: stage.into(),
                    state: state.into(),
                    windows_done: if stage == "extraction" { 0 } else { 1 },
                    windows_total: 1,
                    batches_done: 0,
                    batches_total: if stage == "assessment" { 2 } else { 0 },
                    withheld_count: 0,
                    route:
                        "Route: local/nvidia/nemotron-3-super-120b-a12b:free · attempt 1 · running"
                            .into(),
                    error: String::new(),
                    retryable: false,
                    index_status: "Index pending: 0/0".into(),
                    activity: activity.into(),
                    activity_age_secs: age,
                    artifacts: Vec::new(),
                };
                progress.artifacts =
                    argos_osint_core::investigation::insight_execution::artifact_steps(
                        &progress,
                        &Default::default(),
                    );
                let step = status_step(state, stage, activity, age, false);
                app.select(ModuleId::Intel.index());
                app.intel_articles = vec![article.clone()];
                app.intel_sel = 0;
                app.intel_page = IntelPage::Briefing;
                app.intel_insight_lifecycle = "refreshing".into();
                app.intel_refresh_status = step.clone();
                app.intel_extraction_progress = Some(progress.clone());
                app.compact_pages[ModuleId::Intel.index()] = 0;
                let text = intel_all_pages_text(&mut app, width, height);
                let short = step.split(" · no model activity").next().unwrap();
                assert!(
                    text.contains(&format!("Claims · {short}")),
                    "{name}: {step}\n{text}"
                );
                assert!(!text.contains("Extracting ·"), "old label gone");
                assert!(!text.contains("model activated"));
                if let Some(dir) = &dir {
                    dump_cells(
                        &mut app,
                        dir,
                        &format!("stream-intel-{name}-{width}x{height}"),
                        width,
                        height,
                    );
                }
                let job = JobRow {
                    id: progress.job_id.clone(),
                    app: "intel".into(),
                    operation: "insight_refresh".into(),
                    title: "Assess and refresh investigative insights".into(),
                    state: state.into(),
                    ..Default::default()
                };
                app.select(ModuleId::Jobs.index());
                app.jobs.rows = vec![job.clone()];
                app.jobs.sel = 0;
                app.jobs.detail = Some(JobDetail {
                    job,
                    tasks: vec![TaskRow {
                        id: progress.task_id.clone(),
                        operation: "insight_refresh".into(),
                        state: state.into(),
                        ..Default::default()
                    }],
                    insight_progress: vec![progress],
                    ..Default::default()
                });
                app.jobs.detail_open = true;
                app.jobs.detail_scroll = 0;
                let buffer = render(&mut app, width, height);
                assert!(buffer_text(&buffer).contains(&step), "jobs {name}: {step}");
                if let Some(dir) = &dir {
                    dump_cells(
                        &mut app,
                        dir,
                        &format!("stream-jobs-{name}-{width}x{height}"),
                        width,
                        height,
                    );
                }
            }
            // Supersession: intentional retirement in Jobs detail.
            {
                let mut app = app();
                let progress = Progress {
                    task_id: "insight-stream-task".into(),
                    job_id: "insight-refresh:article:a2:3".into(),
                    retry_epoch: 0,
                    stage: "extraction".into(),
                    state: "superseded".into(),
                    windows_done: 0,
                    windows_total: 1,
                    batches_done: 0,
                    batches_total: 0,
                    withheld_count: 0,
                    route: "Route: local/nvidia/nemotron-3-super-120b-a12b:free · attempt 1 · ok"
                        .into(),
                    error: "Superseded · replaced by insight-refresh:article:a2:4 · source changed"
                        .into(),
                    retryable: false,
                    index_status: "Index: no published memories".into(),
                    activity: String::new(),
                    activity_age_secs: None,
                    artifacts: Vec::new(),
                };
                let job = JobRow {
                    id: progress.job_id.clone(),
                    app: "intel".into(),
                    operation: "insight_refresh".into(),
                    title: "Assess and refresh investigative insights".into(),
                    state: "cancelled".into(),
                    ..Default::default()
                };
                app.select(ModuleId::Jobs.index());
                app.jobs.rows = vec![job.clone()];
                app.jobs.sel = 0;
                app.jobs.detail = Some(JobDetail {
                    job,
                    tasks: vec![TaskRow {
                        id: progress.task_id.clone(),
                        operation: "insight_refresh".into(),
                        state: "superseded".into(),
                        ..Default::default()
                    }],
                    insight_progress: vec![progress],
                    ..Default::default()
                });
                app.jobs.detail_open = true;
                app.jobs.detail_scroll = 0;
                let text = buffer_text(&render(&mut app, width, height));
                assert!(text.contains("Superseded · replaced by"));
                assert!(!text.contains("Extraction error"));
                if let Some(dir) = &dir {
                    dump_cells(
                        &mut app,
                        dir,
                        &format!("stream-jobs-8-superseded-{width}x{height}"),
                        width,
                        height,
                    );
                }
            }
            // Logs: activity transitions recorded by the insight observer.
            let mut app = app();
            for message in [
                "attempt 1 dispatched · local/nvidia/nemotron-3-super-120b-a12b:free · stream · waiting for output",
                "attempt 1 receiving model output",
                "attempt 1 stream complete (stop) · validating",
                "attempt 1 ok · 2814 chars",
            ] {
                argos_osint_core::events::record_event(
                    app.store.connection(),
                    &argos_osint_core::events::NewEvent {
                        severity: Some(argos_osint_core::events::Severity::Info),
                        app: "intel".into(),
                        event_type: "insight.activity".into(),
                        message: message.into(),
                        job_id: "insight-stream-job".into(),
                        task_id: "insight-stream-task".into(),
                        ..Default::default()
                    },
                )
                .unwrap();
            }
            app.select(ModuleId::Logs.index());
            app.reload_logs();
            let buffer = render(&mut app, width, height);
            assert!(buffer_text(&buffer).contains("receiving model output"));
            if let Some(dir) = &dir {
                dump_cells(
                    &mut app,
                    dir,
                    &format!("stream-logs-{width}x{height}"),
                    width,
                    height,
                );
            }
        }
    }

    #[test]
    fn opening_a_brief_admits_one_preparation_chain_without_competing_work() {
        let mut app = app();
        let (_, article) = related_ui_fixture(&mut app);
        app.select(ModuleId::Intel.index());
        app.intel_articles = vec![article.clone()];
        app.intel_sel = 0;
        app.open_intel_briefing();
        let job = app.intel_brief_job.clone().expect("chain admitted");
        let labels: Vec<_> = app.intel_brief_stages.iter().map(|v| v.label).collect();
        assert_eq!(labels, ["Full article", "Claims", "Inferences", "Links"]);
        // Preserve the two roots and their join within the one brief job.
        assert_eq!(app.intel_brief_stages[1].row, "Queued");
        assert_eq!(
            app.intel_brief_stages[2].row,
            "Queued · after Full article, Claims"
        );
        // Reopen attaches to the same chain.
        app.open_intel_briefing();
        assert_eq!(app.intel_brief_job.as_deref(), Some(job.as_str()));
        // No separate report, refresh or body task competes with the chain.
        let competing: i64 = app
            .store
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM argos_tasks WHERE operation IN ('insight_refresh','article_body')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(competing, 0);
        assert!(!app.intel_body_running.contains(&article.id));
    }

    #[test]
    fn extraction_actions_resolve_selected_article_instead_of_stale_display_progress() {
        let mut app = app();
        let (_, article) = related_ui_fixture(&mut app);
        app.select(ModuleId::Intel.index());
        app.intel_articles = vec![article];
        app.intel_sel = 0;
        app.intel_extraction_progress = Some(
            argos_osint_core::investigation::insight_execution::Progress {
                task_id: "other-owner-task".into(),
                job_id: "other-owner-job".into(),
                ..Default::default()
            },
        );
        app.activate_button(ButtonId::IntelExtractionRetry);
        assert_eq!(app.status, "No extraction task");
        app.activate_button(ButtonId::IntelExtractionJob);
        assert_eq!(app.status, "No extraction job");
        assert_eq!(app.module, Some(ModuleId::Intel));
    }

    #[test]
    fn brain_derivation_labels_keep_kind_stance_and_history_separate() {
        let mut app = app();
        let ids = claim_fixture(&app);
        app.select(ModuleId::Brain.index());
        app.reload_memories();
        assert!(app.open_memory_detail(&ids[0]));
        let original = app.selected_insight.clone().unwrap();
        let articles = app.store.atlas_list_articles("run-0412").unwrap();
        let dir = std::env::var("ARGOS_SCREEN_DIR").ok();
        for (kind, lifecycle, relation) in [
            ("claim", "active", "contradicts"),
            ("inference", "pending", "insufficient"),
            ("inference", "previous", "supports"),
        ] {
            let mut insight = original.clone();
            insight.metadata = Some(argos_osint_core::brain_query::InsightMetadata {
                insight_kind: kind.into(),
                lifecycle: lifecycle.into(),
                snapshot_id: "fixture".into(),
                revision: "fixture".into(),
                assessment: argos_osint_core::investigation::insights::Assessment {
                    fidelity: "faithful".into(),
                    relation: relation.into(),
                    threshold_passed: true,
                    rationale: "Original source assertion; world corroboration is separate.".into(),
                    gaps: vec![],
                },
            });
            app.brain_graph = argos_osint_core::recon::build_claim_graph(&insight, &articles);
            app.selected_insight = Some(insight);
            for (width, height) in [(140, 40), (80, 24), (64, 32)] {
                let text = buffer_text(&render(&mut app, width, height));
                assert!(
                    !text.contains("0%"),
                    "new derivations cannot display an invented confidence"
                );
                if width == 140 {
                    assert!(text.contains(lifecycle), "{text}");
                }
                if let Some(dir) = &dir {
                    dump_cells(
                        &mut app,
                        dir,
                        &format!("brain-insight-{kind}-{lifecycle}-{width}x{height}"),
                        width,
                        height,
                    );
                }
            }
        }
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
        render(&mut app, 120, 42);
        // This test covers the explicit source article route. Graph activation
        // is independently covered by the seeded topology entry tests.
        app.set_focus(Target::DetailPath);
        app.open_claim_article("run-brain", "art-brain").unwrap();
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
    fn spaced_lists_reveal_last_items_and_spacers_have_no_record_target() {
        let mut app = app();
        app.module = Some(ModuleId::Recon);
        app.recon_chat = false;
        app.threads = (0..30)
            .map(|i| recon::Thread {
                id: format!("thread-{i}"),
                title: format!("Investigation {i}"),
                created_at: String::new(),
                updated_at: String::new(),
                draft: String::new(),
                scroll: 0,
                recall_insights: false,
            })
            .collect();
        for (width, height) in [
            (160, 50),
            (120, 40),
            (100, 32),
            (80, 24),
            (60, 18),
            (40, 12),
        ] {
            app.scrolls.threads = 0;
            let buffer = render(&mut app, width, height);
            let targets: Vec<_> = app
                .layout
                .borrow()
                .entries
                .iter()
                .filter(|e| matches!(e.target, Target::Thread(_)))
                .cloned()
                .collect();
            if targets.len() >= 2 {
                let first = targets[0].rect;
                let next = targets[1].rect;
                assert_eq!(next.y, first.bottom() + 1);
                assert!(buffer
                    .content
                    .iter()
                    .skip((first.bottom() as usize) * width as usize + first.x as usize)
                    .take(first.width as usize)
                    .all(|cell| cell.symbol() == " "));
                assert!(!matches!(
                    super::super::ui::hit_test(&app, first.x, first.bottom()),
                    Some(Target::Thread(_))
                ));
            }
            app.thread_sel = 29;
            app.move_thread(0);
            render(&mut app, width, height);
            assert!(app
                .layout
                .borrow()
                .entries
                .iter()
                .any(|e| e.target == Target::Thread(29)));
            app.module = None;
            app.set_focus(Target::App(8));
            render(&mut app, width, height);
            assert!(app
                .layout
                .borrow()
                .entries
                .iter()
                .any(|e| e.target == Target::App(8)));
            app.module = Some(ModuleId::Recon);
        }
    }

    #[test]
    fn list_spacing_populated_screens() {
        let mut app = app();
        app.overlay = Overlay::None;
        app.recon_chat = false;
        app.threads = (0..20)
            .map(|i| recon::Thread {
                id: format!("thread-{i}"),
                title: format!("Investigation {i}"),
                created_at: String::new(),
                updated_at: String::new(),
                draft: String::new(),
                scroll: 0,
                recall_insights: false,
            })
            .collect();
        app.memories = (0..20)
            .map(|i| Memory {
                id: format!("memory-{i}"),
                text: format!("Saved claim {i}"),
                category: "fact".into(),
                pinned: false,
                created_at: String::new(),
                source: argos_osint_core::brain::MemorySource {
                    app: "recon".into(),
                    conversation_id: format!("thread-{i}"),
                    message_id: None,
                    reference: None,
                },
            })
            .collect();
        app.jobs.rows = (0..20)
            .map(|i| argos_osint_core::jobs_view::JobRow {
                id: format!("job-{i}"),
                title: format!("Background job {i}"),
                app: "intel".into(),
                state: "completed".into(),
                ..Default::default()
            })
            .collect();
        app.jobs.loaded = true;
        app.intel_articles = (0..20)
            .map(|i| intel_brief_image_article(&format!("article-{i}"), ""))
            .collect();
        app.intel_page = IntelPage::Bulletin;
        for i in 0..3 {
            app.push_log_detail(
                "info",
                format!("Event {i}"),
                "Complete supporting detail wraps together across terminal rows.",
            );
        }
        app.logs.reload(&app.store);
        if let Some(first) = app.logs.rows.first() {
            app.logs.open.insert(first.id.clone());
        }
        let dir = std::env::var("ARGOS_SCREEN_DIR").ok();
        for module in [
            ModuleId::Recon,
            ModuleId::Brain,
            ModuleId::Osint,
            ModuleId::Intel,
            ModuleId::Jobs,
            ModuleId::Logs,
        ] {
            app.module = Some(module);
            app.compact_pages[module.index()] = 0;
            for (width, height) in [
                (160, 50),
                (120, 40),
                (100, 32),
                (80, 24),
                (60, 18),
                (40, 12),
            ] {
                let buffer = render(&mut app, width, height);
                let targets: Vec<_> = app
                    .layout
                    .borrow()
                    .entries
                    .iter()
                    .filter(|entry| match module {
                        ModuleId::Recon => matches!(entry.target, Target::Thread(_)),
                        ModuleId::Brain => matches!(entry.target, Target::Memory(_)),
                        ModuleId::Osint => {
                            matches!(entry.target, Target::Tool(_) | Target::ToolCategory(_))
                        }
                        ModuleId::Intel => matches!(entry.target, Target::IntelArticle(_)),
                        ModuleId::Jobs => matches!(entry.target, Target::JobRow(_)),
                        _ => false,
                    })
                    .cloned()
                    .collect();
                if targets.len() >= 2 {
                    let first = targets[0].rect;
                    let next = targets[1].rect;
                    let gap = usize::from(module != ModuleId::Intel) as u16;
                    assert_eq!(
                        next.y,
                        first.bottom() + gap,
                        "{module:?} at {width}x{height}"
                    );
                    if gap > 0 {
                        for x in first.x..first.right() {
                            assert_eq!(buffer[(x, first.bottom())].symbol(), " ");
                        }
                        let hit = super::super::ui::hit_test(&app, first.x, first.bottom());
                        assert_ne!(hit, Some(targets[0].target));
                        assert_ne!(hit, Some(targets[1].target));
                    }
                }
                if module == ModuleId::Logs {
                    let lines =
                        super::super::logs::row_lines(&app.logs, width.saturating_sub(2) as usize);
                    for (row, line) in lines.iter().enumerate() {
                        if line.spans.is_empty() {
                            assert_eq!(
                                app.logs
                                    .index_at_line(row, width.saturating_sub(2) as usize),
                                None
                            );
                        }
                    }
                }
                if let Some(dir) = &dir {
                    dump_cells(
                        &mut app,
                        dir,
                        &format!("list-spacing-{}-{width}x{height}", module.index()),
                        width,
                        height,
                    );
                }
            }
        }
    }

    #[test]
    fn intel_reload_revisit_and_late_snapshot_keep_generation_ownership() {
        let mut app = app();
        let (_, article) = related_ui_fixture(&mut app);
        app.module = Some(ModuleId::Intel);
        app.intel_articles = vec![article.clone()];
        app.intel_sel = 0;
        let body = app
            .store
            .ensure_article_body(
                &article.id,
                &article.run_id,
                &article.url,
                &article.source_domain,
                &article.source_name,
            )
            .unwrap();
        app.store
            .commit_article_body(
                &body.id,
                "Old usable body",
                "old-hash",
                "complete",
                "complete",
                &article.url,
                "fixture",
                false,
            )
            .unwrap();
        app.store
            .connection()
            .execute(
                "UPDATE article_bodies SET updated_at='2020-01-01T00:00:00Z' WHERE id=?1",
                [&body.id],
            )
            .unwrap();
        app.open_intel_briefing();
        let previous_job = app.intel_brief_job.clone();
        assert!(super::super::ui::intel_body_has_content(&app));
        app.activate_button(ButtonId::IntelBodyRefresh);
        let reload_job = app.intel_brief_job.clone();
        assert_ne!(previous_job, reload_job);
        assert!(app.intel_body_require_commit);
        assert!(!super::super::ui::intel_body_has_content(&app));
        app.leave_intel_briefing();
        app.open_intel_briefing();
        assert_eq!(app.intel_brief_job, reload_job);
        assert!(!super::super::ui::intel_body_has_content(&app));
        let mut obsolete = super::super::db_reads::IntelBriefSnapshot::default();
        obsolete.outputs.insert(
            "intel_brief_body".into(),
            serde_json::json!({"markdown":"Obsolete body"}),
        );
        app.on_work_event(WorkEvent::IntelBriefLoaded {
            key: (
                article.run_id.clone(),
                article.id.clone(),
                app.related_scope,
                previous_job,
                app.intel_related_limit,
            ),
            outcome: Ok(Box::new(obsolete)),
        });
        assert!(!super::super::ui::intel_body_has_content(&app));
        let mut ready = super::super::db_reads::IntelBriefSnapshot::default();
        ready.outputs.insert(
            "intel_brief_body".into(),
            serde_json::json!({"markdown":"Current body"}),
        );
        app.on_work_event(WorkEvent::IntelBriefLoaded {
            key: (
                article.run_id,
                article.id,
                app.related_scope,
                reload_job,
                app.intel_related_limit,
            ),
            outcome: Ok(Box::new(ready)),
        });
        assert!(super::super::ui::intel_body_has_content(&app));
        assert!(!super::super::ui::intel_body_loading(&app));
    }

    #[test]
    fn intel_body_transition_screens_and_quality_states() {
        use super::super::ui::{
            intel_body_loading, intel_body_presentation, IntelBodyPresentation,
        };
        let mut app = app();
        app.module = Some(ModuleId::Intel);
        app.intel_page = IntelPage::Briefing;
        let article = intel_brief_image_article("body-state", "");
        app.intel_articles = vec![article.clone()];
        app.intel_brief_job = Some("body-job".into());
        app.intel_body_owner = Some((article.id.clone(), app.intel_brief_job.clone()));
        let body = app
            .store
            .ensure_article_body(
                &article.id,
                &article.run_id,
                &article.url,
                &article.source_domain,
                &article.source_name,
            )
            .unwrap();
        let dir = std::env::var("ARGOS_SCREEN_DIR").ok();
        for state in [
            "running",
            "committed",
            "committed-overflow",
            "partial",
            "uncertain",
            "read-error",
            "cached-read-error",
            "orphan",
            "retry",
        ] {
            app.intel_body = None;
            app.intel_brief_outputs.clear();
            app.intel_brief_stages.clear();
            app.intel_body_read_error = None;
            app.intel_body_message = "Obsolete retrieval message".into();
            app.intel_body_running.insert(article.id.clone());
            if state == "running" || state == "retry" {
                app.intel_brief_stages = vec![intel_recon::brief_pipeline::StageView {
                    operation: "intel_brief_body".into(),
                    label: "Full article",
                    state: if state == "running" {
                        "running"
                    } else {
                        "retry_scheduled"
                    }
                    .into(),
                    row: state.into(),
                    active: state == "running",
                }];
            }
            if matches!(
                state,
                "committed" | "committed-overflow" | "cached-read-error"
            ) {
                app.intel_brief_outputs.insert("intel_brief_body".into(), serde_json::json!({"markdown":"## Ready article\n\n- First usable finding\n- Second usable finding\n\nA readable committed body."}));
                if state == "committed-overflow" {
                    app.intel_brief_outputs.insert("intel_brief_body".into(), serde_json::json!({"markdown":format!("## Ready article\n\n- First finding\n- Second finding\n\n{}", "A complete committed article paragraph retains qualifications, attribution and Unicode Zoë. ".repeat(200))}));
                }
                // Later stage activity cannot keep Body spinning.
                app.intel_brief_stages
                    .push(intel_recon::brief_pipeline::StageView {
                        operation: "intel_brief_links".into(),
                        label: "Links",
                        state: if state == "committed" {
                            "failed"
                        } else {
                            "running"
                        }
                        .into(),
                        row: if state == "committed" {
                            "Failed · Links unavailable"
                        } else {
                            "Running"
                        }
                        .into(),
                        active: state != "committed",
                    });
            }
            if matches!(state, "partial" | "uncertain" | "orphan") {
                let mut current = body.clone();
                current.state = if state == "orphan" {
                    "running"
                } else {
                    "ready"
                }
                .into();
                current.quality = if state == "orphan" { "complete" } else { state }.into();
                current.quality_rationale = "Limited coverage".into();
                current.body_markdown = if state == "orphan" {
                    "Previous body from an orphaned retrieval"
                } else {
                    "Readable article with quality warning"
                }
                .into();
                app.intel_body = Some(current);
            }
            if state.ends_with("read-error") {
                app.intel_body_read_error = Some("Reader unavailable".into());
            }
            assert_eq!(intel_body_loading(&app), state == "running");
            if matches!(state, "partial" | "uncertain") {
                assert!(matches!(
                    intel_body_presentation(&app),
                    IntelBodyPresentation::Content {
                        warning: Some(_),
                        ..
                    }
                ));
            }
            for (width, height) in [
                (160, 50),
                (120, 40),
                (100, 32),
                (80, 24),
                (60, 18),
                (40, 12),
            ] {
                app.compact_pages[ModuleId::Intel.index()] = 1;
                app.scrolls.intel_brief = 0;
                app.scrolls.intel_full = 0;
                app.set_focus(Target::IntelFullArticle);
                render(&mut app, width, height);
                let geometry = super::super::ui::intel_center_geometry_for(&app).unwrap();
                app.scrolls.intel_brief =
                    (geometry.brief.3 + 3).min(super::super::ui::intel_brief_scroll_max(&app));
                let text = buffer_text(&render(&mut app, width, height));
                assert!(text.contains("full article"), "{state} at {width}x{height}");
                if matches!(
                    state,
                    "committed" | "committed-overflow" | "cached-read-error"
                ) {
                    assert!(
                        text.contains("Ready article"),
                        "{state} at {width}x{height}"
                    );
                }
                if state != "running" {
                    assert!(!text.contains("Retrieving full article"));
                }
                if let Some(dir) = &dir {
                    dump_cells(
                        &mut app,
                        dir,
                        &format!("intel-transition-{state}-{width}x{height}"),
                        width,
                        height,
                    );
                }
            }
        }
    }

    #[test]
    fn intel_body_presentation_uses_commits_and_rejects_obsolete_owners() {
        use super::super::ui::{
            intel_body_loading, intel_body_presentation, IntelBodyPresentation,
        };
        let mut app = app();
        app.intel_articles = vec![intel_brief_image_article("presentation", "")];
        app.intel_sel = 0;
        app.intel_brief_job = Some("job-current".into());
        app.intel_body_running.insert("presentation".into());
        app.intel_body_message = "Retrieving full article…".into();
        assert!(
            !intel_body_loading(&app),
            "messages and legacy flags are not activity"
        );
        app.intel_brief_stages = vec![intel_recon::brief_pipeline::StageView {
            operation: "intel_brief_body".into(),
            label: "Full article",
            state: "running".into(),
            row: "Running".into(),
            active: true,
        }];
        assert!(intel_body_loading(&app));
        let mut snapshot = super::super::db_reads::IntelBriefSnapshot::default();
        snapshot.outputs.insert(
            "intel_brief_body".into(),
            serde_json::json!({"markdown":"Readable committed body"}),
        );
        snapshot.stages = app.intel_brief_stages.clone();
        app.accept_intel_brief_snapshot(snapshot);
        assert!(!intel_body_loading(&app));
        assert!(matches!(
            intel_body_presentation(&app),
            IntelBodyPresentation::Content {
                markdown: "Readable committed body",
                ..
            }
        ));
        assert!(!app.intel_body_running.contains("presentation"));
        assert!(app.intel_body_message.is_empty());
        app.intel_body_read_error = Some("reader failed".into());
        assert!(matches!(
            intel_body_presentation(&app),
            IntelBodyPresentation::Content { .. }
        ));
        app.intel_brief_job = Some("job-reload".into());
        assert!(matches!(
            intel_body_presentation(&app),
            IntelBodyPresentation::Status(_)
        ));
        app.intel_body_owner = None;
        app.intel_brief_outputs.clear();
        assert!(
            matches!(intel_body_presentation(&app), IntelBodyPresentation::Status(reason) if reason.contains("reader failed"))
        );
        app.intel_body_read_error = None;
        app.intel_brief_stages[0].state = "retry_scheduled".into();
        app.intel_brief_stages[0].row = "Retry scheduled".into();
        assert!(!intel_body_loading(&app));
        assert!(
            matches!(intel_body_presentation(&app), IntelBodyPresentation::Status(reason) if reason.contains("Retry scheduled"))
        );
    }

    #[test]
    fn intel_busy_state_is_scoped_to_the_selected_article() {
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
        app.intel_brief_job = Some("busy-job".into());
        app.intel_body_owner = Some(("art-busy".into(), app.intel_brief_job.clone()));
        app.intel_brief_stages = vec![intel_recon::brief_pipeline::StageView {
            operation: "intel_brief_body".into(),
            label: "Full article",
            state: "running".into(),
            row: "Running".into(),
            active: true,
        }];
        app.intel_body_running.insert("art-busy".into());
        assert!(super::super::ui::intel_body_loading(&app));
        app.intel_sel = 1;
        assert!(!super::super::ui::intel_body_loading(&app));
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
        // Spaced Origins scroll independently of the map; reveal the final records.
        super::super::ui::shift_cycle_stats(&mut app, i32::MAX);
        terminal
            .draw(|frame| super::super::ui::draw(frame, &app))
            .unwrap();
        let painted = screen_text(&terminal);
        for name in ["Spain", "Japan", "Brazil", "Australia", "India"] {
            assert!(painted.contains(name), "stats missing {name}");
        }
    }

    #[test]
    fn atlas_live_coverage_shows_article_metadata_units() {
        let mut app = app();
        app.select(ModuleId::Atlas.index());
        app.atlas_page = AtlasPage::Live;
        app.atlas_status = "Classifying batch".into();
        app.atlas_stats.coverage.articles = 12;
        app.atlas_stats.coverage.classified = 8;
        app.atlas_stats.coverage.entities_available = 12;
        let painted = buffer_text(&render(&mut app, 140, 44));
        assert!(painted.contains("Classifications 8/12"), "{painted}");
        assert!(painted.contains("Classifying batch"), "{painted}");
        assert!(
            !painted.contains("Indexed") && !painted.contains("Extracting insights"),
            "{painted}"
        );
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
        assert!(!painted.contains("related context"), "{painted}");
        assert!(painted.contains("Acme Corp"), "{painted}");
        assert!(painted.contains("Source F"), "legacy A1 badge: {painted}");

        // Scroll left column
        let max = super::super::ui::intel_extracted_scroll_max(&app);
        app.scrolls.intel_extracted = max;
        terminal
            .draw(|frame| super::super::ui::draw(frame, &app))
            .unwrap();

        // Actor reviewing loading state
        app.intel_insights_running.insert("art-1".into());
        terminal
            .draw(|frame| super::super::ui::draw(frame, &app))
            .unwrap();
        let painted_actor_rev = screen_text(&terminal);
        assert!(
            painted_actor_rev.contains("Acme Corp")
                && !painted_actor_rev.contains("Reviewing actors"),
            "{painted_actor_rev}"
        );
        app.intel_insights_running.clear();

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
        // Actual buffers for the new lifecycle contract; legacy fixtures above
        // continue to verify compatibility and independent section scrolling.
        app.scrolls.intel_extracted = 0;
        app.intel_claims[0].classification = "claim".into();
        app.intel_claims[0].fingerprint = "v2:claim-fixture".into();
        app.intel_claims[1].fingerprint = "v2:inference-fixture".into();
        app.intel_relations = vec![(
            "v2:claim-fixture".into(),
            "v2:inference-fixture".into(),
            "supports".into(),
        )];
        app.intel_link_explanations.insert(
            ("v2:claim-fixture".into(), "v2:inference-fixture".into()),
            "[supports · assessed] Original launch evidence supports the qualified conclusion."
                .into(),
        );
        let dir = std::env::var("ARGOS_SCREEN_DIR").ok();
        let full = app.intel_claims.clone();
        for status in [
            "active",
            "pending",
            "previous",
            "insufficient_evidence",
            "corroborated",
            "disputed",
        ] {
            app.intel_insight_lifecycle = if matches!(status, "corroborated" | "disputed") {
                "active"
            } else {
                status
            }
            .into();
            app.intel_evidence_confidence = Some(intel_recon::EvidenceConfidence {
                score: Some(match status {
                    "corroborated" => 80,
                    "disputed" => 20,
                    "insufficient_evidence" => 40,
                    _ => 60,
                }),
                claims: 1,
                inferences: 1,
                information_grade: if status == "corroborated" { 2 } else { 6 },
                item_scores: Vec::new(),
                faithful: 1,
                corroborated: usize::from(status == "corroborated"),
                disputed: usize::from(status == "disputed"),
                complete_coverage: status != "insufficient_evidence",
                provisional: status != "corroborated",
            });
            app.intel_claims = full.clone();
            let label = status.replace('_', " ");
            app.intel_claims[0].claim =
                format!("[claim · {label}] Acme reportedly launched Widget.");
            if status == "insufficient_evidence" {
                app.intel_claims.truncate(1);
                app.intel_relations.clear();
            } else {
                app.intel_claims[1].claim = format!("[inference · {status}] Widget may pressure competitors; single-source premise.");
            }
            for (width, height) in [(160, 50), (120, 40), (80, 24), (60, 18), (40, 12)] {
                let buffer = render(&mut app, width, height);
                let text = buffer
                    .content
                    .iter()
                    .map(|c| c.symbol())
                    .collect::<String>();
                assert!(!text.contains("related context"));
                assert!(!text.contains("Brief rating value"));
                assert!(!text.contains("CONFIRMED"));
                if width >= 80 {
                    assert!(
                        text.contains("Source F"),
                        "v2 A6 badge missing at {width}x{height}: {text}"
                    );
                    assert!(text.contains("claims"));
                    assert!(text.find("confidence").unwrap() < text.find("claims").unwrap());
                    let left = app
                        .layout
                        .borrow()
                        .entries
                        .iter()
                        .find(|entry| entry.target == Target::IntelLeftColumn)
                        .unwrap()
                        .rect;
                    assert_eq!(
                        super::super::ui::hit_test(&app, left.x + 1, left.y + 1),
                        Some(Target::IntelLeftColumn)
                    );
                }
                if let Some(dir) = &dir {
                    dump_cells(
                        &mut app,
                        dir,
                        &format!("intel-insights-{status}-{width}x{height}"),
                        width,
                        height,
                    );
                    if width <= 60 {
                        app.scrolls.intel_extracted =
                            super::super::ui::intel_extracted_scroll_max(&app);
                        dump_cells(
                            &mut app,
                            dir,
                            &format!("intel-insights-{status}-scrolled-{width}x{height}"),
                            width,
                            height,
                        );
                        app.scrolls.intel_extracted = 0;
                    }
                }
            }
        }
        app.intel_claims.clear();
        app.intel_evidence_confidence = Some(Default::default());
        let empty = render(&mut app, 160, 50);
        let text = empty
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(!text.contains(" assessment "));
        assert!(!text.contains("Brief rating value"));
        assert!(text.contains("N/A"));
        if let Some(dir) = &dir {
            dump_cells(&mut app, dir, "intel-confidence-empty-160x50", 160, 50);
            app.intel_evidence_confidence = None;
            app.intel_metadata_present = true;
            app.intel_publisher_prior = Some(publisher_prior_fixture());
            app.intel_insight_lifecycle = "legacy/unassessed".into();
            let initial = render(&mut app, 160, 50);
            assert!(initial
                .content
                .iter()
                .map(|c| c.symbol())
                .collect::<String>()
                .contains("Brief rating"));
            for (width, height) in [(160, 50), (120, 40), (100, 32), (80, 24), (60, 18)] {
                dump_cells(
                    &mut app,
                    dir,
                    &format!("intel-confidence-initial-{width}x{height}"),
                    width,
                    height,
                );
            }
        }
    }

    fn publisher_prior_fixture() -> argos_osint_core::atlas_metadata::PublisherPrior {
        use argos_osint_core::osint::{
            wikipedia_rsp::SourceReliabilityObservation, SourceReliability,
        };
        let mut observation =
            SourceReliabilityObservation::from_lookup("news.example", "Baltic Wire", None);
        observation.reliability = "B".into();
        observation.reliability_label = SourceReliability::B.label().into();
        observation.rsp_status = "gr".into();
        observation.rsp_status_label = "generally reliable".into();
        observation.listed = true;
        observation.rsp_url="https://en.wikipedia.org/wiki/Wikipedia:Reliable_sources/Perennial_sources#Baltic_Wire".into();
        argos_osint_core::atlas_metadata::PublisherPrior {
            observation,
            tool_id: "wikipedia_source_reliability".into(),
            matched_domain: Some("news.example".into()),
            fetched_at: "2026-10-12T00:00:00Z".into(),
            score: Some(85.0),
            version: "publisher-rsp-v1".into(),
            stale: false,
        }
    }

    #[test]
    fn persisted_publisher_prior_is_independent_of_evidence_and_process_cache() {
        let mut app = app();
        app.module = Some(ModuleId::Intel);
        app.intel_page = IntelPage::Briefing;
        app.intel_articles
            .push(intel_brief_image_article("prior", ""));
        app.intel_metadata_present = true;
        app.intel_publisher_prior = Some(publisher_prior_fixture());
        app.intel_evidence_confidence = None;
        let text = buffer_text(&render(&mut app, 160, 50));
        assert!(text.contains("60/100"), "{text}");
        assert!(text.contains("Information 6"), "{text}");
        assert!(text.contains("Brief rating"), "{text}");
        assert!(!text.contains("20/100"), "{text}");
        app.intel_publisher_prior = None;
        let text = buffer_text(&render(&mut app, 160, 50));
        assert!(
            text.contains("Source F") && text.contains("N/A"),
            "metadata without a usable prior must not borrow a legacy/cache grade: {text}"
        );
    }

    fn intel_brief_image_article(id: &str, image_url: &str) -> AtlasArticleRow {
        AtlasArticleRow {
            run_id: "r1".into(),
            id: id.into(),
            title: "Brief focus article with an image column".into(),
            description: "First sentence of the deterministic preview stays readable beside the image column. Second sentence keeps the prose wide enough to wrap across several rows. Third sentence makes the section taller than the image so the image stays vertically centered. Fourth sentence leaves the reader readable at every supported viewport size.".into(),
            url: "https://example.com/article".into(),
            country: "us".into(),
            source_name: "Test Source".into(),
            source_domain: "example.com".into(),
            author: String::new(),
            image_url: image_url.into(),
            published_at: "2026-10-08T12:00:00Z".into(),
            provider: "newsapi".into(),
            temperature: 0.8,
            category: "stability".into(),
            seen_at: String::new(),
        }
    }

    #[test]
    fn dump_article_insights_screens() {
        let mut app = app();
        app.module = Some(ModuleId::Intel);
        app.intel_page = IntelPage::Briefing;
        let article = intel_brief_image_article("actors", "");
        app.store.atlas_upsert_article(&article).unwrap();
        app.intel_articles = vec![article.clone()];
        app.intel_publisher_prior = Some(publisher_prior_fixture());
        app.intel_metadata_present = true;
        for i in 0..14 {
            let name = format!("Actor {i} · Zoë");
            app.intel_actors
                .push(argos_osint_core::atlas_metadata::ArticleActor {
                    id: format!("actor-{i}"),
                    type_hint: "person".into(),
                    display_name: name.clone(),
                    spellings: vec![name.clone()],
                    mentions: vec![argos_osint_core::atlas_metadata::ArticleMention {
                        type_hint: "person".into(),
                        comparison_key: name.to_lowercase(),
                        field: "title".into(),
                        start_byte: 0,
                        end_byte: name.len(),
                        original_text: name,
                    }],
                });
        }
        app.intel_actor_state = "complete".into();
        app.intel_refresh_status = "Waiting · complete article details required".into();
        let body = app
            .store
            .ensure_article_body(
                &article.id,
                &article.run_id,
                &article.url,
                &article.source_domain,
                &article.source_name,
            )
            .unwrap();
        let dir = std::env::var("ARGOS_SCREEN_DIR").ok();
        for (width, height) in [
            (160, 50),
            (120, 40),
            (100, 32),
            (80, 24),
            (60, 18),
            (40, 20),
        ] {
            app.compact_pages[ModuleId::Intel.index()] = 0;
            app.scrolls.intel_extracted = 0;
            let buffer = render(&mut app, width, height);
            let text = buffer_text(&buffer);
            if width >= 80 {
                assert!(text.contains("60/100"));
                assert!(!text.contains(" assessment "));
            }
            if let Some(dir) = &dir {
                dump_cells(
                    &mut app,
                    dir,
                    &format!("intel-actors-without-claims-{width}x{height}"),
                    width,
                    height,
                );
            }
            app.scrolls.intel_extracted = super::super::ui::intel_extracted_scroll_max(&app);
            let _ = render(&mut app, width, height);
            if let Some(dir) = &dir {
                dump_cells(
                    &mut app,
                    dir,
                    &format!("intel-actors-tail-{width}x{height}"),
                    width,
                    height,
                );
            }
            app.compact_pages[ModuleId::Intel.index()] = 1;
            app.scrolls.intel_brief = 0;
            app.set_focus(Target::IntelFullArticle);
            let geometry = super::super::ui::intel_center_geometry_for(&app).unwrap();
            app.scrolls.intel_brief =
                (geometry.brief.3 + 3).min(super::super::ui::intel_brief_scroll_max(&app));
            app.intel_brief_job = Some("fixture-body-job".into());
            app.intel_body_owner = Some((article.id.clone(), app.intel_brief_job.clone()));
            app.intel_brief_stages = vec![intel_recon::brief_pipeline::StageView {
                operation: "intel_brief_body".into(),
                label: "Full article",
                state: "running".into(),
                row: "Running".into(),
                active: true,
            }];
            app.intel_body_running.insert(article.id.clone());
            if let Some(dir) = &dir {
                dump_cells(
                    &mut app,
                    dir,
                    &format!("intel-body-bordered-loading-{width}x{height}"),
                    width,
                    height,
                );
            }
            app.intel_body_running.clear();
            app.intel_brief_stages[0].state = "failed".into();
            app.intel_brief_stages[0].row = "Failed · Acquisition failed".into();
            app.intel_brief_stages[0].active = false;
            app.intel_body_message = "Acquisition failed · retained previous insights".into();
            app.intel_body = Some(body.clone());
            if let Some(dir) = &dir {
                dump_cells(
                    &mut app,
                    dir,
                    &format!("intel-body-bordered-error-{width}x{height}"),
                    width,
                    height,
                );
            }
            let text="A complete retained article paragraph with qualifications, attribution and Unicode Zoë. ".repeat(120);
            app.store
                .commit_article_body(
                    &body.id,
                    &text,
                    "fixture",
                    "complete",
                    "fixture",
                    &article.url,
                    "fixture",
                    false,
                )
                .unwrap();
            app.intel_body = app.store.article_body_for_article(&article.id).unwrap();
            app.intel_body_message.clear();
            app.scrolls.intel_full = 0;
            if let Some(dir) = &dir {
                dump_cells(
                    &mut app,
                    dir,
                    &format!("intel-body-bordered-long-{width}x{height}"),
                    width,
                    height,
                );
            }
            if width >= 60 {
                let _ = render(&mut app, width, height);
                let body_rect = app
                    .layout
                    .borrow()
                    .entries
                    .iter()
                    .find(|e| e.target == Target::IntelFullArticle)
                    .expect("Full article focus target")
                    .rect;
                assert_eq!(
                    super::super::ui::hit_test(&app, body_rect.x + 1, body_rect.y + 1),
                    Some(Target::IntelFullArticle)
                );
                assert!(super::super::ui::focus_order(&app).contains(&Target::IntelFullArticle));
                app.set_focus(Target::IntelFullArticle);
                let parent = app.scrolls.intel_brief;
                app.move_vertical(1);
                assert!(app.scrolls.intel_full > 0);
                assert_eq!(app.scrolls.intel_brief, parent);
                super::super::ui::page(&mut app, 1);
                assert!(app.scrolls.intel_full >= 6);
                assert_eq!(app.scrolls.intel_brief, parent);
                app.scrolls.intel_full = 0;
                let _ = render(&mut app, width, height);
                let more = app
                    .layout
                    .borrow()
                    .entries
                    .iter()
                    .find(|e| e.target == Target::Button(ButtonId::IntelBodyMore))
                    .expect("Full article See more must be reachable")
                    .rect;
                assert_eq!(
                    super::super::ui::hit_test(&app, more.x + 1, more.y + 1),
                    Some(Target::Button(ButtonId::IntelBodyMore))
                );
                assert!(super::super::ui::focus_order(&app)
                    .contains(&Target::Button(ButtonId::IntelBodyMore)));
                let saved = app.scrolls.intel_brief;
                app.activate_button(ButtonId::IntelBodyMore);
                assert!(
                    matches!(&app.overlay,Overlay::Block {title,body} if title=="Full article" && body==&text)
                );
                app.handle_key(crossterm::event::KeyCode::Esc.into());
                assert_eq!(app.scrolls.intel_brief, saved);
                app.scrolls.intel_full = u16::MAX;
                let _ = render(&mut app, width, height);
                assert!(
                    !app.layout
                        .borrow()
                        .entries
                        .iter()
                        .any(|e| e.target == Target::Button(ButtonId::IntelBodyMore)),
                    "See more disappears at the bottom"
                );
            }
            app.intel_body = None;
            app.intel_brief_stages.clear();
            app.intel_body_message.clear();
            app.scrolls.intel_full = 0;
            if let Some(dir) = &dir {
                dump_cells(
                    &mut app,
                    dir,
                    &format!("intel-body-bordered-empty-{width}x{height}"),
                    width,
                    height,
                );
            }
            app.scrolls.intel_brief = 0;
            app.scrolls.intel_full = 0;
            app.scrolls.intel_extracted = 0;
        }
        app.module = Some(ModuleId::Jobs);
        app.jobs.reload(&app.store);
        app.set_focus(Target::Button(ButtonId::JobsClearCompleted));
        for (width, height) in [(160, 50), (80, 24), (60, 18)] {
            let buffer = render(&mut app, width, height);
            assert!(buffer_text(&buffer).contains("Clear completed"));
            let target = app
                .layout
                .borrow()
                .entries
                .iter()
                .find(|entry| entry.target == Target::Button(ButtonId::JobsClearCompleted))
                .unwrap()
                .rect;
            assert_eq!(
                super::super::ui::hit_test(
                    &app,
                    target.x + target.width / 2,
                    target.y + target.height / 2
                ),
                Some(Target::Button(ButtonId::JobsClearCompleted))
            );
            if let Some(dir) = &dir {
                dump_cells(
                    &mut app,
                    dir,
                    &format!("jobs-clear-status-{width}x{height}"),
                    width,
                    height,
                );
            }
        }
    }

    /// Select Intel, install one brief-article fixture and return the center
    /// reader geometry for the requested viewport.
    fn intel_brief_image_geometry(
        app: &mut App,
        article: AtlasArticleRow,
        width: u16,
        height: u16,
        reader_page: usize,
    ) -> crate::tui::ui::IntelCenterGeometry {
        app.select(ModuleId::Intel.index());
        app.intel_page = IntelPage::Briefing;
        app.intel_articles = vec![article];
        app.intel_sel = 0;
        app.compact_pages[ModuleId::Intel.index()] = reader_page;
        app.screen = Rect::new(0, 0, width, height);
        crate::tui::ui::intel_center_geometry_for(app)
            .unwrap_or_else(|| panic!("brief layout missing at {width}x{height}"))
    }

    #[test]
    fn intel_brief_image_column_sits_beside_brief_content_and_centers_vertically() {
        let mut app = app();
        let geo = intel_brief_image_geometry(
            &mut app,
            intel_brief_image_article("art-1", "https://example.com/image.png"),
            160,
            50,
            0,
        );
        let (image_x, image_y, image_w, image_h) = geo.image;
        let (brief_x, brief_y, brief_w, brief_h) = geo.brief;
        let (text_x, text_y, text_w, text_h) = geo.brief_text;
        let inner_w = brief_w.saturating_sub(2) as usize;
        let columns = image_w as usize + 1 + text_w as usize;
        let image_bottom = image_y + image_h as i32;
        let section_bottom = brief_y + brief_h as i32 - 1;
        // The image column stays inside the brief article section.
        assert!(image_x > brief_x, "{geo:?}");
        assert!(image_x + image_w <= brief_x + brief_w - 1, "{geo:?}");
        assert!(image_y > brief_y, "{geo:?}");
        assert!(image_bottom <= section_bottom, "{geo:?}");
        // The image column is strictly left of the brief prose.
        assert!(image_x + image_w <= text_x, "{geo:?}");
        // Image, gutter and prose fill the section inner width.
        assert_eq!(columns, inner_w, "{geo:?}");
        assert!(text_w >= 24, "{geo:?}");
        // Image and prose are vertically centered inside the section.
        let section_mid = brief_y + brief_h as i32 / 2;
        let image_mid = image_y + image_h as i32 / 2;
        let text_mid = text_y + text_h as i32 / 2;
        assert!((image_mid - section_mid).abs() <= 1, "{geo:?}");
        assert!((text_mid - section_mid).abs() <= 1, "{geo:?}");
        assert!((14..=40).contains(&image_w), "{geo:?}");
        assert!((5..=12).contains(&image_h), "{geo:?}");
    }

    #[test]
    fn intel_brief_image_column_adapts_and_collapses_by_viewport() {
        let mut app = app();
        for (width, height) in [(160u16, 50u16), (120, 40), (100, 32), (80, 24), (60, 18)] {
            let geo = intel_brief_image_geometry(
                &mut app,
                intel_brief_image_article("art-1", "https://example.com/image.png"),
                width,
                height,
                1,
            );
            assert!(geo.image.2 > 0, "{width}x{height}: {geo:?}");
            assert!((5..=12).contains(&geo.image.3), "{width}x{height}: {geo:?}");
            assert!(geo.brief_text.2 >= 24, "{width}x{height}: {geo:?}");
            match width {
                160 => assert!(geo.image.2 >= 20, "{width}x{height}: {geo:?}"),
                100 => assert!(
                    (21..=23).contains(&geo.image.2),
                    "{width}x{height}: {geo:?}"
                ),
                80 | 60 => assert_eq!(geo.image.3, 5, "{width}x{height}: {geo:?}"),
                _ => {}
            }
        }
        // The image column collapses at 40x20; the prose keeps the inner width.
        let geo = intel_brief_image_geometry(
            &mut app,
            intel_brief_image_article("art-1", "https://example.com/image.png"),
            40,
            20,
            1,
        );
        assert_eq!(geo.image.2, 0, "{geo:?}");
        assert_eq!(geo.image.3, 0, "{geo:?}");
        assert_eq!(geo.brief_text.0, geo.brief.0 + 1, "{geo:?}");
        assert_eq!(geo.brief_text.2, geo.brief.2 - 2, "{geo:?}");
    }

    #[test]
    fn intel_brief_text_keeps_full_width_without_image() {
        let mut app = app();
        for (label, image_url) in [("absent", ""), ("invalid", "file:///etc/passwd")] {
            let geo = intel_brief_image_geometry(
                &mut app,
                intel_brief_image_article("art-1", image_url),
                160,
                50,
                0,
            );
            assert_eq!(geo.image.2, 0, "{label}: {geo:?}");
            assert_eq!(geo.image.3, 0, "{label}: {geo:?}");
            assert_eq!(geo.brief_text.0, geo.brief.0 + 1, "{label}: {geo:?}");
            assert_eq!(geo.brief_text.2, geo.brief.2 - 2, "{label}: {geo:?}");
        }
    }

    #[test]
    fn intel_brief_image_column_renders_beside_content_in_buffer() {
        let mut app = app();
        let geo = intel_brief_image_geometry(
            &mut app,
            intel_brief_image_article("art-1", "https://example.com/image.png"),
            160,
            50,
            0,
        );
        let image_area = crate::tui::ui::article_image_area(&app);
        if image_area.width > 0 && image_area.height > 0 {
            app.article_images
                .fixture("art-1", "https://example.com/image.png", image_area);
        }
        let buffer = render(&mut app, 160, 50);
        let painted = buffer_text(&buffer);
        assert!(painted.contains(" brief article "), "{painted}");
        let (image_x, image_y, image_w, image_h) = geo.image;
        let (text_x, text_y, text_w, text_h) = geo.brief_text;
        let rows = buffer.area.height as i32;
        let cols = buffer.area.width;
        let mut image_blocks = 0;
        for row in image_y..(image_y + image_h as i32).min(rows) {
            for col in image_x..(image_x + image_w).min(cols) {
                if matches!(buffer[(col, row as u16)].symbol(), "▀" | "▄") {
                    image_blocks += 1;
                }
            }
        }
        assert!(image_blocks > 0, "no image glyphs in {image_area:?}");
        let mut text_blocks = 0;
        for row in text_y..(text_y + text_h as i32).min(rows) {
            for col in text_x..(text_x + text_w).min(cols) {
                if matches!(buffer[(col, row as u16)].symbol(), "▀" | "▄") {
                    text_blocks += 1;
                }
            }
        }
        assert_eq!(text_blocks, 0, "image glyphs inside {image_area:?}");
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
        for (width, height) in [
            (160u16, 50u16),
            (120, 40),
            (100, 32),
            (60, 18),
            (80, 24),
            (40, 20),
            (100, 24),
        ] {
            let mut app = app();
            app.select(ModuleId::System.index());
            seed_profile(&mut app);
            let text = buffer_text(&render(&mut app, width, height));
            assert!(
                text.contains("[p]"),
                "{width}x{height}: the filter strip must render: {text}"
            );
            assert!(
                text.contains("Overview"),
                "{width}x{height}: the tab switch must render: {text}"
            );
            // The status strip names the reviewed inventory instead of guessing.
            assert!(
                text.contains("20") || width < 80,
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
            text.contains("Overview"),
            "the tab strip survives a narrow viewport: {text}"
        );
        assert!(
            text.contains("wider") || text.contains("[p]"),
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
        assert_eq!(app.overlay, Overlay::None);
        assert_eq!(app.profile.tab, crate::tui::profile::SystemTab::Configs);
        let keyboard_focus = app.focus;
        assert!(
            matches!(keyboard_focus, Target::Field(FieldId::ProfileExportPath)),
            "the export tab focuses its path field: {keyboard_focus:?}"
        );

        // Mouse: the same field is reachable by clicking, and the hit test
        // resolves it to the same target.
        click(&mut app, Target::Field(FieldId::ProfileExportPath));
        assert_eq!(app.focus, keyboard_focus);

        // Tab visits Export before Import, in visual order.
        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(app.focus, Target::ConfigAction(0));
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
        app.set_focus(Target::Field(FieldId::ProfileImportEditor));
        assert_eq!(app.profile.tab, crate::tui::profile::SystemTab::Configs);

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
    fn profile_configs_page_uses_the_body_and_has_visible_actions() {
        let mut app = app();
        app.select(ModuleId::System.index());
        app.handle_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));
        let text = buffer_text(&render(&mut app, 120, 40));
        assert_eq!(app.overlay, Overlay::None);
        assert_eq!(app.profile.tab, crate::tui::profile::SystemTab::Configs);
        assert!(
            text.contains("Import JSON")
                && text.contains("Verify")
                && text.contains("Save and apply"),
            "{text}"
        );
        assert!(hit(&app, Target::ConfigAction(0)));
        assert!(hit(&app, Target::ConfigAction(1)));
        assert!(!hit(&app, Target::ConfigAction(2)));
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

    fn analytics_fixture() -> argos_osint_core::profile_stats::ProfileSnapshot {
        use argos_osint_core::profile_stats::*;
        let mut snapshot = ProfileSnapshot {
            captured_at: "2026-10-11T00:00:00Z".into(),
            observed_since: "2026-10-08T00:00:00Z".into(),
            ..Default::default()
        };
        snapshot.status.collection_health = "ok".into();
        snapshot.intel.volume = (0..24)
            .map(|hour| VolumeBucket {
                bucket: format!("2026-10-10T{hour:02}"),
                total: hour,
                by_tag: vec![("politics".into(), hour), ("economy".into(), hour)],
                untagged: 0,
            })
            .collect();
        snapshot.tools.top_tools = (0..140)
            .map(|i| ToolCount {
                tool_id: format!("tool_{i:03}"),
                category: "web".into(),
                invocations: i + 1,
                remote: i + 1,
                ..Default::default()
            })
            .collect();
        snapshot.intel.distinct_new_articles = Some(276);
        snapshot.intel.body_eligible_articles = Some(276);
        snapshot.intel.body_available_articles = Some(240);
        snapshot.intel.enrichment = vec![
            EnrichmentRow {
                tag: "politics".into(),
                articles: 140,
                body_pct: Some(88.0),
                claims_pct: Some(75.0),
                report_pct: Some(64.0),
                ..Default::default()
            },
            EnrichmentRow {
                tag: "economy".into(),
                articles: 90,
                body_pct: Some(86.0),
                claims_pct: Some(81.0),
                report_pct: Some(60.0),
                ..Default::default()
            },
        ];
        snapshot.intel.reports = vec![
            ReportModeRow {
                mode: "verify".into(),
                completed: 72,
                partial: 6,
                failed: 3,
                waiting: 2,
                blocked: 1,
                n: 81,
                ..Default::default()
            },
            ReportModeRow {
                mode: "full_assessment".into(),
                completed: 45,
                partial: 9,
                failed: 6,
                n: 60,
                ..Default::default()
            },
        ];
        snapshot.intel.publishers = vec![
            PublisherCount {
                domain: "reuters.com".into(),
                articles: 85,
                share: 30.8,
                top3_concentration: 70.3,
                ..Default::default()
            },
            PublisherCount {
                domain: "apnews.com".into(),
                articles: 65,
                share: 23.6,
                top3_concentration: 70.3,
                ..Default::default()
            },
            PublisherCount {
                domain: "bbc.com".into(),
                articles: 44,
                share: 15.9,
                top3_concentration: 70.3,
                ..Default::default()
            },
        ];
        snapshot.intel.freshness = FreshnessHistogram {
            buckets: vec![
                ("<1h".into(), 84),
                ("1–6h".into(), 96),
                ("6–24h".into(), 60),
                ("1–7d".into(), 20),
                (">7d".into(), 6),
            ],
            missing: 8,
            future: 2,
        };
        snapshot.recon.outcomes = (0..24)
            .map(|hour| ReconOutcomeBucket {
                bucket: format!("2026-10-10T{hour:02}"),
                mode: "verify".into(),
                completed_with_evidence: 2 + (hour % 5),
                completed_zero_evidence: hour % 3,
                partial: hour % 2,
                failed: u32::from(hour % 7 == 0),
                cancelled: u32::from(hour % 11 == 0),
            })
            .collect();
        snapshot.recon.directives = vec![
            DirectiveResolution {
                mode: "verify".into(),
                answered: 184,
                partial: 32,
                unresolved: 16,
                blocked: 8,
                unknown: 4,
                n: 244,
            },
            DirectiveResolution {
                mode: "full_assessment".into(),
                answered: 62,
                partial: 8,
                unresolved: 6,
                blocked: 2,
                unknown: 2,
                n: 80,
            },
        ];
        snapshot.recon.stages = vec![
            StageDurationRow {
                stage: "discovery".into(),
                mode: "verify".into(),
                mean_exec_ms: Some(3800),
                mean_wait_ms: Some(700),
                n: 42,
            },
            StageDurationRow {
                stage: "picker".into(),
                mode: "verify".into(),
                mean_exec_ms: Some(1800),
                mean_wait_ms: Some(2200),
                n: 42,
            },
            StageDurationRow {
                stage: "synthesis".into(),
                mode: "verify".into(),
                mean_exec_ms: Some(6500),
                mean_wait_ms: Some(1200),
                n: 42,
            },
        ];
        snapshot.recon.diversity = vec![
            DiversityRow {
                category: "web".into(),
                eligible_scopes: 52,
                scopes_with_two_attempted: 48,
                scopes_with_two_successful: 42,
                eligible: true,
                top_shortfall_reason: "One independent source".into(),
                ..Default::default()
            },
            DiversityRow {
                category: "social".into(),
                eligible_scopes: 34,
                scopes_with_two_attempted: 29,
                scopes_with_two_successful: 25,
                eligible: true,
                top_shortfall_reason: "Provider unavailable".into(),
                ..Default::default()
            },
        ];
        snapshot.recon.unresolved = (0..8)
            .map(|i| UnresolvedDirective {
                run_id: format!("run-{i:03}"),
                mode: "verify".into(),
                label: format!("d{} · Verify publisher ownership", i + 1),
                reason: "Second independent source unavailable".into(),
                evidence_count: 1,
                last_progress: "2026-10-10T11:30:00Z".into(),
                next_action: "Open investigation and inspect sources".into(),
            })
            .collect();
        snapshot.atlas.backlog = vec![
            BacklogRow {
                stage: "classification".into(),
                unit: "article".into(),
                queued: 24,
                blocked: 2,
                latest_error_category: "configuration".into(),
                ..Default::default()
            },
            BacklogRow {
                stage: "entities".into(),
                unit: "article".into(),
                queued: 12,
                running: 3,
                ..Default::default()
            },
            BacklogRow {
                stage: "publishers".into(),
                unit: "article".into(),
                waiting: 6,
                ..Default::default()
            },
            BacklogRow {
                stage: "related".into(),
                unit: "cycle".into(),
                queued: 1,
                oldest_pending_ms: Some(120000),
                ..Default::default()
            },
            BacklogRow {
                stage: "legacy · index".into(),
                unit: "memory".into(),
                queued: 6,
                oldest_pending_ms: Some(120000),
                ..Default::default()
            },
        ];
        snapshot.atlas.cycle_outcomes = (0..24)
            .map(|hour| CycleOutcomeBucket {
                bucket: format!("2026-10-10T{hour:02}"),
                completed: 2 + hour % 3,
                partial: hour % 2,
                failed: u32::from(hour % 8 == 0),
                cancelled: 0,
            })
            .collect();
        snapshot.atlas.temperature_changes = vec![
            TemperatureChange {
                origin: "US".into(),
                previous_temperature: Some(45.0),
                current_temperature: Some(52.0),
                delta: Some(7.0),
                articles: 32,
                comparable: true,
                label: "warming".into(),
                ..Default::default()
            },
            TemperatureChange {
                origin: "GB".into(),
                previous_temperature: Some(48.0),
                current_temperature: Some(43.0),
                delta: Some(-5.0),
                articles: 24,
                comparable: true,
                label: "cooling".into(),
                ..Default::default()
            },
        ];
        snapshot.models.capacity_available = true;
        snapshot.models.capacity = vec![argos_osint_core::provider_metrics::CapacityRow {
            provider: "openrouter".into(),
            quota_group: "account".into(),
            scope: "all".into(),
            sends_60s: 32,
            effective_rpm: Some(60),
            pace_per_min: Some(32.0),
            active: 3,
            max_concurrency: Some(8),
            queued: 2,
            oldest_wait_ms: Some(1400),
            quota_source: "configured".into(),
            ..Default::default()
        }];
        snapshot.models.period_summary = ModelPeriodSummary {
            sends: 620,
            terminal_operations: 280,
            final_failures: 8,
            successful_execution_p95_ms: Some(3200),
            successful_execution_n: 570,
            queue_p95_ms: Some(850),
            queue_n: 610,
        };
        snapshot.models.performance = vec![
            ProviderModelRow {
                provider: "openrouter".into(),
                model: "".into(),
                role: "recon".into(),
                sends: 620,
                completed_attempts: 590,
                attempt_error_pct: Some(4.8),
                final_operation_failure_pct: Some(2.9),
                mean_exec_ms: Some(1400),
                p95_exec_ms: Some(3200),
                n: 570,
                ..Default::default()
            },
            ProviderModelRow {
                provider: "openrouter".into(),
                model: "fixture-model".into(),
                role: "recon".into(),
                sends: 620,
                is_model_row: true,
                n: 570,
                ..Default::default()
            },
        ];
        snapshot.models.latency = (0..24)
            .map(|hour| LatencyBucket {
                bucket: format!("2026-10-10T{hour:02}"),
                p50_ms: if hour == 9 {
                    None
                } else {
                    Some(1000 + (hour % 5) * 150)
                },
                p95_ms: if hour == 9 || hour == 17 {
                    None
                } else {
                    Some(2400 + (hour % 7) * 180)
                },
                n: if hour == 17 { 3 } else { 24 },
                ..Default::default()
            })
            .collect();
        snapshot.models.queue_delay = (0..24)
            .map(|hour| QueueBucket {
                bucket: format!("2026-10-10T{hour:02}"),
                p50_ms: Some(150 + (hour % 5) * 50),
                p95_ms: Some(500 + (hour % 6) * 75),
                n: 24,
            })
            .collect();
        snapshot.tools.reliability = (0..240)
            .map(|i| ReliabilityRow {
                tool_id: format!("tool_{i:03}"),
                category: "web".into(),
                invocations: i + 1,
                wire_requests: i + 1,
                cache_hit_pct: Some(10.0),
                verified_zero_pct: Some(12.0),
                error_pct: Some(3.0),
                mean_ms: Some(430),
                p95_ms: Some(1200),
                dominant_trigger: "recon".into(),
                ..Default::default()
            })
            .collect();
        snapshot.tools.engine_health = vec![EngineHealthRow {
            engine: "google".into(),
            fetches: 120,
            valid_serps: 98,
            verified_zero: 12,
            challenge: 4,
            parser_mismatch: 2,
            transport_failure: 4,
            usable_per_fetch: Some(91.7),
            last_success: "2026-10-10T11:58:00Z".into(),
            parser_version: "fixture-1".into(),
            ..Default::default()
        }];
        snapshot.tools.evidence = vec![EvidenceRow {
            tool_id: "firecrawl_search".into(),
            category: "web".into(),
            successful_nonempty: 98,
            with_evidence: 84,
            acceptance_pct: Some(85.7),
            distinct_evidence: 120,
            cited_by_completed_reports: 72,
            coverage_n: 98,
            nonadditive: true,
        }];
        snapshot.tools.failure_causes = vec![
            FailureCauseRow {
                cause: "transport".into(),
                invocations: 12,
                share: 60.0,
                rank: 1,
            },
            FailureCauseRow {
                cause: "blocked".into(),
                invocations: 8,
                share: 40.0,
                rank: 2,
            },
        ];
        snapshot.recon.unresolved_live_total = snapshot.recon.unresolved.len() as u64;
        snapshot.tools.logical_calls = snapshot
            .tools
            .reliability
            .iter()
            .map(|row| row.invocations)
            .sum();
        snapshot.tools.remote_errors = 20;
        snapshot.tools.eligible_remote_calls = 120;
        snapshot.attention = snapshot
            .recon
            .unresolved
            .iter()
            .map(|row| AttentionRow {
                app: "Recon".into(),
                owner_kind: "run".into(),
                owner_id: row.run_id.clone(),
                item_id: row.label.clone(),
                reason: row.reason.clone(),
                age_ms: Some(1800000),
                action: "Open investigation".into(),
            })
            .collect();
        snapshot
    }

    #[test]
    fn profile_refresh_requests_and_publication_require_visible_analytics() {
        let mut app = app();
        app.profile.accept_snapshot(analytics_fixture());
        let previous = app.profile.snapshot.clone();
        let at = Instant::now() - Duration::from_secs(25);
        app.profile.loaded_at = Some(at);
        for (module, tab, overlay) in [
            (
                None,
                super::super::profile::SystemTab::Overview,
                Overlay::None,
            ),
            (
                Some(ModuleId::Recon),
                super::super::profile::SystemTab::Overview,
                Overlay::None,
            ),
            (
                Some(ModuleId::System),
                super::super::profile::SystemTab::System,
                Overlay::None,
            ),
            (
                Some(ModuleId::System),
                super::super::profile::SystemTab::Overview,
                Overlay::Configs,
            ),
        ] {
            app.module = module;
            app.profile.tab = tab;
            app.overlay = overlay;
            app.reload_profile();
            assert!(
                !app.profile.loading,
                "hidden analytics must not start a read"
            );
            app.profile.loading = true;
            assert!(!app.on_work_event(WorkEvent::ProfileLoaded {
                generation: app.profile.generation,
                filters: app.profile.filters.clone(),
                outcome: Ok(Box::default()),
                options: Vec::new(),
            }));
            assert!(!app.profile.loading);
            assert_eq!(app.profile.snapshot, previous);
            assert_eq!(app.profile.loaded_at, Some(at));
        }
        app.module = Some(ModuleId::System);
        app.profile.tab = super::super::profile::SystemTab::Overview;
        app.overlay = Overlay::None;
        assert!(app.profile_analytics_visible());
        assert!(app.profile.due(), "overdue refresh resumes on return");
        assert!(app.on_work_event(WorkEvent::ProfileLoaded {
            generation: app.profile.generation,
            filters: app.profile.filters.clone(),
            outcome: Ok(Box::default()),
            options: Vec::new(),
        }));
        assert_ne!(app.profile.snapshot, previous);
        assert!(!app.profile.due());
        app.profile.loaded_at = Some(Instant::now() - Duration::from_secs(19));
        assert!(!app.profile.due());
        app.profile.loaded_at = Some(Instant::now() - Duration::from_secs(21));
        assert!(app.profile.due());
    }

    #[test]
    fn profile_all_20_reports_have_registered_keyboard_and_mouse_routes() {
        let mut app = app();
        app.select(ModuleId::System.index());
        app.profile.accept_snapshot(analytics_fixture());
        let mut ids = HashSet::new();
        for section in 1..=5 {
            app.handle_key(KeyEvent::new(
                KeyCode::Char(char::from_digit(section, 10).unwrap()),
                KeyModifiers::NONE,
            ));
            for index in 0..app.profile.focused_widgets().len() {
                app.profile.focus = index;
                app.set_focus(Target::ProfileCard(index));
                render(&mut app, 120, 40);
                assert!(hit(&app, Target::ProfileCard(index)));
                app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
                let id = app.profile.report.unwrap();
                ids.insert(id);
                render(&mut app, 120, 40);
                assert!(super::super::ui::focus_order(&app).contains(&Target::ProfileReport));
                app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
                assert_eq!(app.profile.focus, index);
                assert!(app.profile.report.is_none());
            }
        }
        assert_eq!(ids.len(), 20);
    }

    #[test]
    fn profile_scrolls_past_64_records_and_global_chords_keep_their_owners() {
        let mut app = app();
        app.select(ModuleId::System.index());
        app.profile.accept_snapshot(analytics_fixture());
        app.profile.report = Some("tools.reliability");
        let first = buffer_text(&render(&mut app, 160, 50));
        assert!(first.contains("Rows 1–10 / 240"), "{first}");
        app.profile.report = Some("models.latency");
        let compact = buffer_text(&render(&mut app, 60, 18));
        assert!(compact.contains("Rows 1–2 / 24"), "{compact}");
        app.profile.report = Some("tools.reliability");
        app.set_focus(Target::ProfileReport);
        render(&mut app, 120, 40);
        app.set_focus(Target::ProfileTable(0));
        crate::tui::profile::scroll(&mut app, 100_000);
        let text = buffer_text(&render(&mut app, 120, 40));
        assert!(text.contains("tool_239"), "{text}");
        assert!(app.profile.table_offsets[&("tools.reliability", 0)] > 200);
        assert_eq!(app.profile.report_scroll.offset, 0);
        let first_offset = app.profile.table_offsets[&("tools.reliability", 0)];
        app.set_focus(Target::ProfileTable(1));
        assert_eq!(
            app.profile.table_offsets[&("tools.reliability", 0)],
            first_offset
        );
        app.handle_key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL));
        assert_ne!(app.overlay, Overlay::None);
    }

    #[test]
    fn profile_filters_period_picker_and_obsolete_results() {
        let mut app = app();
        app.select(ModuleId::System.index());
        app.profile.accept_snapshot(analytics_fixture());
        app.profile.options = vec![("category".into(), vec!["web".into()])];
        app.handle_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(
            app.profile.filters.period,
            argos_osint_core::profile_stats::Period::H7
        );
        app.handle_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE));
        for expected in ["provider", "role", "mode", "tool", "category"] {
            app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
            assert_eq!(app.profile.filter_popup, Some(expected));
        }
        // Typing c belongs to the field and does not clear the period/dimensions.
        app.handle_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.profile.filters.category, "web");
        let before = app.profile.snapshot.clone();
        app.profile.loading = true;
        app.on_work_event(WorkEvent::ProfileLoaded {
            generation: 0,
            filters: Default::default(),
            outcome: Ok(Box::default()),
            options: Vec::new(),
        });
        assert_eq!(app.profile.snapshot, before);
        assert!(!app.profile.loading);
        app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE));
        assert!(app.profile.filters.category.is_empty());
        assert_eq!(
            app.profile.filters.period,
            argos_osint_core::profile_stats::Period::H7
        );
    }

    #[test]
    fn compact_home_and_recon_pages_keep_controls_reachable() {
        let mut app = app();
        app.go_home();
        render(&mut app, 40, 12);
        let order = super::super::ui::focus_order(&app);
        for index in 0..9 {
            assert!(order.contains(&Target::App(index)), "missing app {index}");
        }
        assert!(order.contains(&Target::Field(FieldId::Composer)));
        app.select(ModuleId::Recon.index());
        app.recon_chat = true;
        app.recon_context_enabled = true;
        render(&mut app, 80, 24);
        assert!(super::super::ui::focus_order(&app).contains(&Target::PaneTab(1)));
        app.activate_target(Target::PaneTab(1));
        render(&mut app, 80, 24);
        assert!(super::super::ui::focus_order(&app).contains(&Target::ReconContext));
    }

    #[test]
    fn dashboard_page_scroll_focus_and_detail_restore_at_minimum_size() {
        let mut app = app();
        app.select(ModuleId::System.index());
        app.profile.accept_snapshot(analytics_fixture());
        render(&mut app, 60, 18);
        app.set_focus(Target::ProfileCard(0));
        app.handle_key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE));
        let offset = app.profile.grid_scroll.offset;
        assert!(offset > 0);
        render(&mut app, 60, 18);
        assert_eq!(app.profile.grid_scroll.offset, offset);
        app.handle_key(KeyEvent::new(KeyCode::Char('m'), KeyModifiers::NONE));
        assert!(app.profile.report.is_some());
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.profile.grid_scroll.offset, offset);
        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        render(&mut app, 60, 18);
        assert_eq!(app.focus, Target::ProfileCard(1));
        assert!(hit(&app, Target::ProfileCard(1)));
    }
    #[test]
    fn custom_period_validates_timezone_and_order_then_applies() {
        let mut app = app();
        app.select(ModuleId::System.index());
        app.profile.accept_snapshot(analytics_fixture());
        app.handle_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE));
        super::super::profile::activate(&mut app, Target::ProfileChoice(4));
        app.profile.filter_edit = "2026-10-10T12:00:00 | 2026-10-09T12:00:00".into();
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(app.profile.picker_error.is_some());
        assert!(app.profile.period_popup);
        app.profile.filter_edit = "2026-10-09T12:00:00-04:00 | 2026-10-10T12:00:00-04:00".into();
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(!app.profile.period_popup);
        assert!(matches!(
            app.profile.filters.period,
            argos_osint_core::profile_stats::Period::Custom { .. }
        ));
    }
    #[test]
    fn dashboard_mouse_selection_survives_reordered_snapshot() {
        let mut app = app();
        app.select(ModuleId::System.index());
        let fixture = analytics_fixture();
        app.profile.accept_snapshot(fixture.clone());
        super::super::profile::activate(&mut app, Target::ProfileApp(2));
        render(&mut app, 160, 50);
        click(&mut app, Target::ProfilePanelRow(4, 2));
        assert_eq!(app.profile.selected_row["recon.unresolved"], 2);
        let mut next = fixture;
        next.recon.unresolved.reverse();
        app.profile.accept_snapshot(next);
        assert_eq!(app.profile.selected_row["recon.unresolved"], 5);
        let layout = super::super::ui::focus_order(&app);
        assert!(layout.contains(&Target::ProfileCard(4)));
    }

    #[test]
    fn attention_opens_the_exact_provider_quota_scope() {
        let mut app = app();
        app.select(ModuleId::System.index());
        let mut fixture = analytics_fixture();
        let mut second = fixture.models.capacity[0].clone();
        second.quota_group = "other-account".into();
        fixture.models.capacity.push(second);
        fixture.attention = vec![argos_osint_core::profile_stats::AttentionRow {
            owner_kind: "provider_scope".into(),
            owner_id: "openrouter all".into(),
            item_id: "other-account".into(),
            ..Default::default()
        }];
        app.profile.accept_snapshot(fixture);
        app.profile.report = Some("summary.attention");
        super::super::profile::handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        );
        assert_eq!(app.profile.report, Some("models.capacity"));
        assert_eq!(app.profile.selected_row["models.capacity"], 1);
        assert_eq!(app.focus, Target::ProfileReport);
    }

    #[test]
    fn provider_groups_fold_without_losing_selection_on_refresh() {
        let mut app = app();
        app.select(ModuleId::System.index());
        let fixture = analytics_fixture();
        app.profile.accept_snapshot(fixture.clone());
        super::super::profile::activate(&mut app, Target::ProfileApp(4));
        app.profile.focus = 1;
        let before = buffer_text(&render(&mut app, 160, 50));
        assert!(before.contains("▾"));
        super::super::profile::handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE),
        );
        let folded = buffer_text(&render(&mut app, 160, 50));
        assert!(folded.contains("▸"));
        let selected = app.profile.selected_row["models.performance"];
        app.profile.accept_snapshot(fixture);
        assert_eq!(app.profile.selected_row["models.performance"], selected);
        super::super::profile::handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE),
        );
        assert!(buffer_text(&render(&mut app, 160, 50)).contains("▾"));
        app.profile.report = Some("models.performance");
        render(&mut app, 160, 50);
        click(&mut app, Target::ProfileAction(5));
        assert!(app.profile.collapsed_providers["openrouter"]);
        app.set_focus(Target::ProfileAction(5));
        super::super::profile::handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        );
        assert!(!app.profile.collapsed_providers["openrouter"]);
    }

    #[test]
    fn analytics_viewports_and_data_states() {
        let mut app = app();
        app.select(ModuleId::System.index());
        app.profile.accept_snapshot(analytics_fixture());
        for (width, height) in [
            (160, 50),
            (120, 40),
            (100, 32),
            (60, 18),
            (80, 24),
            (40, 20),
            (100, 24),
        ] {
            for (name, page, report) in [
                ("summary", 0, None),
                ("intel", 1, None),
                ("recon", 2, None),
                ("atlas", 3, None),
                ("models", 4, None),
                ("tools", 5, None),
                ("report", 1, Some("intel.enrichment")),
                ("small-sample", 4, Some("models.latency")),
            ] {
                super::super::profile::activate(&mut app, Target::ProfileApp(page));
                app.profile.report = report;
                let buffer = render(&mut app, width, height);
                assert!(!super::super::ui::focus_order(&app).is_empty());
                let rows: Vec<_> = app
                    .layout
                    .borrow()
                    .entries
                    .iter()
                    .filter(|entry| {
                        report.is_none() && matches!(entry.target, Target::ProfilePanelRow(_, _))
                    })
                    .cloned()
                    .collect();
                for pair in rows.windows(2) {
                    let (Target::ProfilePanelRow(panel, _), Target::ProfilePanelRow(next, _)) =
                        (pair[0].target, pair[1].target)
                    else {
                        unreachable!()
                    };
                    if panel != next {
                        continue;
                    }
                    assert_eq!(
                        pair[1].rect.y,
                        pair[0].rect.bottom() + 1,
                        "{name} at {width}x{height}"
                    );
                    for x in pair[0].rect.x..pair[0].rect.right() {
                        assert_eq!(
                            buffer[(x, pair[0].rect.bottom())].symbol(),
                            " ",
                            "{name} panel {panel}"
                        );
                    }
                    assert!(!matches!(
                        super::super::ui::hit_test(&app, pair[0].rect.x, pair[0].rect.bottom()),
                        Some(Target::ProfilePanelRow(_, _))
                    ));
                }
                if let Ok(dir) = std::env::var("ARGOS_SCREEN_DIR") {
                    std::fs::create_dir_all(&dir).unwrap();
                    let cells:Vec<_>=(0..height).map(|y|(0..width).map(|x|{let cell=&buffer[(x,y)];serde_json::json!({"s":cell.symbol(),"fg":format!("{:?}",cell.fg),"bg":format!("{:?}",cell.bg),"b":cell.modifier.contains(ratatui::style::Modifier::BOLD),"u":false})}).collect::<Vec<_>>()).collect();
                    std::fs::write(
                        format!("{dir}/profile-{name}-{width}x{height}.json"),
                        serde_json::json!({"width":width,"height":height,"cells":cells})
                            .to_string(),
                    )
                    .unwrap();
                }
            }
        }
        app.profile.report = None;
        app.profile.all_apps = true;
        app.profile.error = Some("fixture read failed".into());
        let stale = buffer_text(&render(&mut app, 80, 24));
        assert!(stale.contains("Stale") && stale.contains("fixture read failed"));
        if let Ok(dir) = std::env::var("ARGOS_SCREEN_DIR") {
            dump_cells(&mut app, &dir, "profile-stale-160x50", 160, 50);
        }
        app.profile.error = None;
        app.profile.accept_snapshot(Default::default());
        if let Ok(dir) = std::env::var("ARGOS_SCREEN_DIR") {
            dump_cells(&mut app, &dir, "profile-empty-160x50", 160, 50);
        }
        app.profile.snapshot = None;
        app.profile.loading = true;
        assert!(buffer_text(&render(&mut app, 80, 24)).contains("Loading"));
        if let Ok(dir) = std::env::var("ARGOS_SCREEN_DIR") {
            dump_cells(&mut app, &dir, "profile-loading-80x24", 80, 24);
        }
        app.profile.loading = false;
        app.profile.tab = super::super::profile::SystemTab::System;
        if let Ok(dir) = std::env::var("ARGOS_SCREEN_DIR") {
            dump_cells(&mut app, &dir, "profile-system-80x24", 80, 24);
        }
    }
    #[test]
    fn overhaul_tabs_focus_without_activating_and_configs_suspend_reads() {
        let mut app = app();
        app.select(ModuleId::System.index());
        render(&mut app, 160, 50);
        app.set_focus(Target::ProfileTab(1));
        assert_eq!(app.profile.tab, crate::tui::profile::SystemTab::Overview);
        app.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        assert_eq!(app.focus, Target::ProfileTab(2));
        assert_eq!(app.profile.tab, crate::tui::profile::SystemTab::Overview);
        app.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
        assert_eq!(app.profile.tab, crate::tui::profile::SystemTab::Configs);
        assert!(!app.profile_analytics_visible());
        let order = crate::tui::ui::focus_order(&app);
        assert!(!order
            .iter()
            .any(|target| matches!(target, Target::ProfileCard(_))));
        assert!(order.contains(&Target::ConfigAction(1)));
        assert!(!order.contains(&Target::ConfigAction(2)));
    }

    #[test]
    fn overhaul_palette_recency_and_disabled_dispatch() {
        let mut app = app();
        app.run_palette("tools");
        app.run_palette("home");
        app.run_palette("tools");
        assert_eq!(app.palette_recent, vec!["tools", "home"]);
        assert_eq!(app.palette_items()[0].category, "Recently used");
        app.select(ModuleId::Recon.index());
        let before = app.palette_recent.clone();
        app.run_palette("cancel");
        assert_eq!(app.palette_recent, before);
        assert!(app
            .palette_items()
            .iter()
            .all(|item| item.category != "Intel"));
    }

    #[test]
    fn overhaul_tool_category_collapse_preserves_identity_and_search_state() {
        let mut app = app();
        app.select(ModuleId::Osint.index());
        let category = osint::registry()[app.tool_sel].category;
        app.activate_target(Target::ToolCategory(category));
        assert_eq!(app.focus, Target::ToolCategory(category));
        let selected = app.tool_sel;
        let expanded = app.tool_expanded.clone();
        app.osint_search = osint::registry()[selected].id.into();
        assert!(
            crate::tui::tool_catalog::rows(&app.osint_search, &app.tool_expanded)
                .iter()
                .any(|(target, _)| *target == Target::Tool(selected))
        );
        app.osint_search.clear();
        assert_eq!(app.tool_expanded, expanded);
        assert_eq!(app.tool_sel, selected);
    }

    #[test]
    fn overhaul_configs_control_enter_never_commits() {
        let mut app = app();
        app.select(ModuleId::System.index());
        crate::tui::profile::activate(&mut app, Target::ProfileAction(3));
        app.set_focus(Target::Field(FieldId::ProfileImportEditor));
        app.edit_paste("{\n  \"schema_version\": 999\n}");
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::CONTROL));
        assert!(app.profile_config.plan.is_none());
        assert!(app.profile_config.import_status.is_none());
        app.config_action(1);
        assert!(app.profile_config.import_error.is_some());
        assert!(app.profile_config.plan.is_none());
    }

    #[test]
    fn overhaul_browser_url_validation() {
        assert!(valid_browser_url("https://example.com/article?q=a&b=c"));
        for value in [
            "",
            "file:///etc/passwd",
            "javascript:alert(1)",
            "--help",
            "https://",
        ] {
            assert!(!valid_browser_url(value));
        }
    }

    #[test]
    fn dump_overhaul_screens() {
        let Ok(dir) = std::env::var("ARGOS_SCREEN_DIR") else {
            return;
        };
        std::fs::create_dir_all(&dir).unwrap();
        for (width, height) in [
            (160, 50),
            (120, 40),
            (100, 32),
            (80, 24),
            (60, 18),
            (40, 20),
        ] {
            let mut app = app();
            dump_cells(
                &mut app,
                &dir,
                &format!("home-{width}x{height}"),
                width,
                height,
            );
            for module in ModuleId::ALL {
                app.select(module.index());
                dump_cells(
                    &mut app,
                    &dir,
                    &format!("app-{}-{width}x{height}", module.index()),
                    width,
                    height,
                );
            }
            app.select(ModuleId::Osint.index());
            for page in 0..3 {
                app.compact_pages[ModuleId::Osint.index()] = page;
                dump_cells(
                    &mut app,
                    &dir,
                    &format!("tools-page-{page}-{width}x{height}"),
                    width,
                    height,
                );
            }
            app.select(ModuleId::System.index());
            crate::tui::profile::activate(&mut app, Target::ProfileAction(3));
            app.set_focus(Target::Field(FieldId::ProfileImportEditor));
            app.profile_config
                .insert("{\n  \"schema_version\": 1,\n  \"providers\": []\n}");
            dump_cells(
                &mut app,
                &dir,
                &format!("configs-draft-{width}x{height}"),
                width,
                height,
            );
            app.profile_config.insert(", invalid");
            app.profile_config.validate();
            dump_cells(
                &mut app,
                &dir,
                &format!("configs-error-{width}x{height}"),
                width,
                height,
            );
            app.open_palette();
            dump_cells(
                &mut app,
                &dir,
                &format!("palette-{width}x{height}"),
                width,
                height,
            );
            app.overlay = Overlay::Help;
            dump_cells(
                &mut app,
                &dir,
                &format!("help-{width}x{height}"),
                width,
                height,
            );
            app.overlay = Overlay::Block {
                title: "Full record".into(),
                body: (0..200)
                    .map(|i| format!("Record {i}: Unicode 東京 🛰 and complete data\n"))
                    .collect(),
            };
            dump_cells(
                &mut app,
                &dir,
                &format!("record-{width}x{height}"),
                width,
                height,
            );
            for (name, overlay) in [
                (
                    "memories",
                    Overlay::Memories {
                        message_id: "fixture-answer".into(),
                    },
                ),
                ("choice", Overlay::Choice(ChoiceKind::Model)),
                ("fallback", Overlay::AddFallback),
                (
                    "resume",
                    Overlay::ResumeSession(Box::new(LastViewSession {
                        module: Some("recon".into()),
                        intel_page: None,
                        intel_article_id: None,
                        intel_article_title: None,
                        recon_thread_id: Some("fixture-thread".into()),
                        recon_thread_title: Some("Saved investigation".into()),
                        memory_id: None,
                        memory_title: None,
                        atlas_page: None,
                        osint_tool_id: None,
                        providers_page: None,
                        updated_at: "2026-10-10T12:00:00Z".into(),
                    })),
                ),
            ] {
                app.overlay = overlay;
                dump_cells(
                    &mut app,
                    &dir,
                    &format!("overlay-{name}-{width}x{height}"),
                    width,
                    height,
                );
            }
            app.overlay = Overlay::None;
            app.module = Some(ModuleId::Recon);
            app.recon_chat = true;
            app.selected_thread = Some("fixture-thread".into());
            app.messages = vec![
                recon::Message { id: "fixture-query".into(), thread_id: "fixture-thread".into(), sequence: 1, role: "user".into(), content: "Investigate the publisher ownership. Include evidence and coverage. 東京".into(), run_id: None, created_at: String::new() },
                recon::Message { id: "fixture-answer".into(), thread_id: "fixture-thread".into(), sequence: 2, role: "assistant".into(), content: "## Findings\n\nTwo independent records agree on ownership.\n\n- Captured evidence E1\n- Verify the remaining coverage gap\n\n### Coverage\n\nThe retained records support this conclusion.".into(), run_id: None, created_at: String::new() },
            ];
            dump_cells(
                &mut app,
                &dir,
                &format!("recon-transcript-{width}x{height}"),
                width,
                height,
            );
            app.chat_follow = false;
            dump_cells(
                &mut app,
                &dir,
                &format!("recon-history-{width}x{height}"),
                width,
                height,
            );
            app.overlay = Overlay::None;
            app.select(ModuleId::Intel.index());
            app.intel_articles.push(AtlasArticleRow {
                run_id: "fixture".into(), id: "fixture-article".into(), title: "Article imagery and browser actions keep a long headline fully visible by wrapping across several preview rows instead of truncating it".into(), description: "Deterministic article preview. Image opens the article URL. Secondary stories remain reachable.".into(),
                url: "https://example.com/article".into(), country: "US".into(), source_name: "Fixture".into(), source_domain: "example.com".into(), published_at: "2026-10-10T12:00:00Z".into(), provider: "fixture".into(), temperature: 0.5,
                category: "world".into(), seen_at: "2026-10-10T12:00:00Z".into(), author: "Fixture author".into(), image_url: "https://example.com/image.png".into(),
            });
            app.intel_sel = app.intel_articles.len() - 1;
            render(&mut app, width, height);
            let image_area = crate::tui::ui::article_image_area(&app);
            if image_area.width > 0 && image_area.height > 0 {
                app.article_images.fixture(
                    "fixture-article",
                    "https://example.com/image.png",
                    image_area,
                );
            }
            dump_cells(
                &mut app,
                &dir,
                &format!("intel-image-{width}x{height}"),
                width,
                height,
            );
            app.intel_page = IntelPage::Briefing;
            app.compact_pages[ModuleId::Intel.index()] = 1;
            let image_area = crate::tui::ui::article_image_area(&app);
            if image_area.width > 0 && image_area.height > 0 {
                app.article_images.fixture(
                    "fixture-article",
                    "https://example.com/image.png",
                    image_area,
                );
            }
            dump_cells(
                &mut app,
                &dir,
                &format!("intel-brief-image-{width}x{height}"),
                width,
                height,
            );
        }
    }
    #[test]
    fn overhaul_transcript_scroll_exceeds_u16_and_keeps_history_anchor() {
        let mut app = app();
        app.module = Some(ModuleId::Recon);
        app.recon_chat = true;
        app.recon_context_enabled = false;
        app.selected_thread = Some("long-history".into());
        app.messages = vec![recon::Message {
            id: "answer".into(),
            thread_id: "long-history".into(),
            sequence: 1,
            role: "assistant".into(),
            content: (0..70_000).map(|i| format!("Row {i} 東京\n")).collect(),
            run_id: None,
            created_at: String::new(),
        }];
        app.chat_follow = true;
        render(&mut app, 80, 24);
        crate::tui::ui::normalize(&mut app);
        assert!(app.scrolls.chat > 65_535);
        app.chat_follow = false;
        app.scrolls.chat = 66_000;
        let before = app.scrolls.chat;
        app.messages[0].content.push_str("Incoming tail\n");
        crate::tui::ui::normalize(&mut app);
        assert!(!app.chat_follow);
        assert_eq!(app.scrolls.chat, before);
        let text = buffer_text(&render(&mut app, 80, 24));
        assert!(text.contains("Jump to latest"));
    }

    #[test]
    fn dump_topology_density_and_entries_screens() {
        let directory = std::env::var("ARGOS_SCREEN_DIR").ok();
        if let Some(dir) = &directory {
            std::fs::create_dir_all(dir).unwrap();
        }
        for (width, height) in [
            (160, 50),
            (120, 40),
            (100, 32),
            (80, 24),
            (60, 18),
            (40, 20),
        ] {
            let mut app = app();
            let capture = |app: &mut App, name: &str| {
                let buffer = render(app, width, height);
                if let Some(dir) = &directory {
                    dump_cells(app, dir, &format!("{name}-{width}x{height}"), width, height);
                }
                buffer
            };
            capture(&mut app, "density-home");
            app.module = Some(ModuleId::Atlas);
            app.palette_recent = [
                "intel", "atlas", "brain", "recon", "jobs", "logs", "tools", "models",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect();
            app.open_palette();
            capture(&mut app, "density-palette");
            app.overlay = Overlay::None;
            app.atlas_page = AtlasPage::Live;
            app.compact_pages[ModuleId::Atlas.index()] = 2;
            app.atlas_feed = (0..24)
                .map(|i| atlas::FeedArticle {
                    id: format!("article-{i}"),
                    title: format!("Harbor and freight headline {i}"),
                    description: "Retained source metadata".into(),
                    url: format!("https://example.test/article/{i}"),
                    country: "us".into(),
                    source_name: "Harbor Ledger".into(),
                    source_domain: "example.test".into(),
                    author: String::new(),
                    image_url: String::new(),
                    published_at: "2026-10-15T12:00:00Z".into(),
                    provider: "fixture".into(),
                    temperature: 0.5,
                    seen_at: "2026-10-15T12:00:00Z".into(),
                    category: "economic".into(),
                })
                .collect();
            capture(&mut app, "density-atlas-feed");
            app.atlas_page = AtlasPage::Runs;
            app.atlas_runs = (0..24)
                .map(|i| AtlasRunRow {
                    id: format!("cycle-{i}"),
                    state: "completed".into(),
                    phase: 5,
                    cursor_json: "{}".into(),
                    stats_json: "{}".into(),
                    note: String::new(),
                    started_at: "2026-10-15T12:00:00Z".into(),
                    finished_at: "2026-10-15T12:01:00Z".into(),
                })
                .collect();
            capture(&mut app, "density-atlas-cycles");
            app.atlas_news = true;
            app.atlas_news_run = "cycle-0".into();
            app.atlas_articles = (0..24)
                .map(|i| AtlasArticleRow {
                    run_id: "cycle-0".into(),
                    id: format!("article-{i}"),
                    title: format!("Retained harbor article {i}"),
                    source_name: "Harbor Ledger".into(),
                    country: "us".into(),
                    category: "economic".into(),
                    published_at: "2026-10-15T12:00:00Z".into(),
                    ..Default::default()
                })
                .collect();
            capture(&mut app, "density-atlas-articles");
            app.module = Some(ModuleId::Intel);
            app.intel_articles = app.atlas_articles.clone();
            app.intel_category = "economic".into();
            app.intel_page = IntelPage::Bulletin;
            let _ = capture(&mut app, "density-intel-bulletin");
            let targets: Vec<_> = app
                .layout
                .borrow()
                .entries
                .iter()
                .filter(|e| matches!(e.target, Target::IntelArticle(_)))
                .cloned()
                .collect();
            if targets.len() > 1 {
                assert_eq!(targets[1].rect.y, targets[0].rect.bottom());
            }
            app.intel_page = IntelPage::Briefing;
            app.compact_pages[ModuleId::Intel.index()] = 0;
            app.intel_brief_outputs.insert("intel_brief_claims".into(),serde_json::json!({"claims":[{"id":"c1","text":"The Bay Authority awarded the harbor contract to Aster Logistics. Unicode 東京 keeps the complete wrapped record actionable.","attributed_to":"Harbor Ledger","modality":"reported"},{"id":"c2","text":"Freight capacity declined in the latest report."}],"withheld":2}));
            app.intel_actors = (0..12)
                .map(|i| argos_osint_core::atlas_metadata::ArticleActor {
                    id: format!("source-actor-{i}"),
                    display_name: format!("Aster Logistics division {i}"),
                    type_hint: "organization".into(),
                    spellings: vec![],
                    mentions: vec![],
                })
                .collect();
            capture(&mut app, "intel-topology-claims");
            app.intel_brief_outputs.clear();
            capture(&mut app, "intel-topology-actors-no-claims");
            app.set_focus(Target::IntelTopologyActor(11));
            capture(&mut app, "intel-topology-actor-tail");
        }
    }

    #[test]
    fn topology_intel_claim_uses_raw_identity_and_clicked_origin() {
        let mut app = app();
        app.module = Some(ModuleId::Intel);
        app.intel_page = IntelPage::Briefing;
        app.intel_articles = vec![intel_brief_image_article("attributed-claim", "")];
        app.intel_brief_outputs.insert("intel_brief_claims".into(), serde_json::json!({"claims":[{"id":"c1","text":"Exact publication text","attributed_to":"Aster Logistics","modality":"reported"}]}));
        assert!(app.intel_topology_claim_entries()[0].1.contains("reported"));
        app.focus = Target::IntelPreview;
        app.activate_target(Target::IntelTopologyClaim(0));
        assert!(
            matches!(&app.topology_state.seed, Some(argos_osint_core::brain_topology::TopologySeed::RetainedClaim { text, local_id, .. }) if text == "Exact publication text" && local_id == "c1")
        );
        app.activate_button(ButtonId::TopologyBack);
        assert_eq!(app.focus, Target::IntelTopologyClaim(0));
    }

    #[test]
    fn topology_graph_wheel_and_pages_pan_the_same_canvas() {
        let mut app = topology_fixture();
        render(&mut app, 160, 50);
        let rect = app
            .layout
            .borrow()
            .entries
            .iter()
            .find(|entry| entry.target == Target::TopologyGraph)
            .unwrap()
            .rect;
        let point = (rect.y..rect.bottom())
            .flat_map(|y| (rect.x..rect.right()).map(move |x| (x, y)))
            .find(|(x, y)| super::super::ui::hit_test(&app, *x, *y) == Some(Target::TopologyGraph))
            .unwrap();
        super::super::ui::scroll_at(&mut app, point.0, point.1, 1);
        assert_eq!(app.topology_state.pan.1, -3);
        app.set_focus(Target::TopologyGraph);
        super::super::ui::page(&mut app, 1);
        assert!(app.topology_state.pan.1 < -3);
        app.activate_button(ButtonId::TopologyFitGraph);
        assert_eq!(app.topology_state.pan, (0, 0));
    }

    fn topology_fixture() -> App {
        use argos_osint_core::brain_topology::{
            TopologyEdge, TopologyEdgeKind as E, TopologyNode, TopologyNodeKind as K, TopologySeed,
            TopologySnapshot,
        };
        let mut app = app();
        app.module = Some(ModuleId::Brain);
        app.brain_list_mode = BrainListMode::Topology;
        app.topology_origin_label = "Brain · claim path".into();
        let seed = TopologySeed::Claim("c1".into());
        let records = [
            ("c1", K::Claim, "Harbor contract shifted", 0),
            ("a1", K::Entity, "Aster Logistics", 1),
            ("a2", K::Entity, "Bay Authority", 1),
            ("t1", K::Topic, "Port procurement", 1),
            ("r1", K::Article, "Tender report", 1),
            ("i1", K::Inference, "Supplier exposure", 1),
            ("r2", K::Article, "Freight review", 1),
            ("s1", K::Source, "Harbor Ledger", 2),
            ("c2", K::Claim, "Capacity fell 18%", 2),
        ];
        let nodes = records
            .into_iter()
            .map(|(id, kind, label, min_hops)| TopologyNode {
                id: id.into(),
                kind,
                label: label.into(),
                detail: format!(
                    "{label}. Retained fictional fixture record with exact source revision."
                ),
                subtype: if kind == K::Topic {
                    "derived article tag"
                } else {
                    "fixture"
                }
                .into(),
                status: "retained".into(),
                is_root: id == "c1",
                article_id: if kind == K::Article {
                    id.into()
                } else {
                    String::new()
                },
                run_id: "cycle-42".into(),
                published_at: "2026-10-15T12:00:00Z".into(),
                cycle: Some("cycle-42".into()),
                member_ids: if kind == K::Entity {
                    vec![id.into()]
                } else {
                    vec![]
                },
                evidence_count: 1,
                passages: vec!["Exact retained source passage · revision demo-r7".into()],
                min_hops,
                revision: "demo-r7".into(),
                predecessor: (id != "c1").then(|| "c1".into()),
                topology_seed: None,
            })
            .collect::<Vec<_>>();
        let edges = [
            ("c1", "a1", E::Concerns, "concerns"),
            ("c1", "a2", E::Concerns, "concerns"),
            ("c1", "t1", E::Contains, "tagged"),
            ("c1", "r1", E::Source, "sourced from"),
            ("i1", "c1", E::Premise, "premise"),
            ("c1", "r2", E::Related, "possible context"),
            ("r1", "s1", E::Source, "published by"),
            ("c2", "a1", E::Concerns, "concerns"),
            ("c2", "r2", E::Source, "sourced from"),
        ]
        .into_iter()
        .enumerate()
        .map(|(i, (from, to, kind, label))| TopologyEdge {
            id: format!("edge-{i}"),
            from: from.into(),
            to: to.into(),
            kind,
            label: label.into(),
            status: if kind == E::Related {
                "tentative"
            } else {
                "recorded"
            }
            .into(),
            member_ids: vec![],
            receipts: vec!["Article revision demo-r7 · retained evidence".into()],
            rationale: "Exact relationship receipt; no inferred identity or corroboration.".into(),
        })
        .collect::<Vec<_>>();
        app.topology_state.seed = Some(seed.clone());
        app.topology_state.selected_node_id = Some("c1".into());
        app.topology_state.snapshot = Some(TopologySnapshot {
            seed: Some(seed),
            root_id: "c1".into(),
            root_label: "Harbor contract shifted".into(),
            root_kind: Some(K::Claim),
            total_nodes: nodes.len(),
            total_edges: edges.len(),
            discovered_nodes: nodes.len(),
            discovered_edges: edges.len(),
            nodes,
            edges,
            exploration_exhausted: true,
            ..Default::default()
        });
        app
    }

    #[test]
    fn topology_keyboard_respects_filters_and_exits_edge_inspection() {
        let mut app = topology_fixture();
        render(&mut app, 160, 50);
        app.topology_state.filter_actor = false;
        app.topology_state.filter_inference = false;
        app.topology_state.filter_sources = false;
        app.activate_target(Target::TopologyEdge(0));
        app.topology_state.inspector_scroll = 100;
        app.handle_topology_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(app.topology_state.selected_node_id.as_deref(), Some("c2"));
        assert!(app.topology_state.selected_edge_id.is_none());
        assert_eq!(app.topology_state.inspector_scroll, 0);
        let Target::TopologyNode(index) = app.focus else {
            panic!("node focus required")
        };
        assert_eq!(
            app.topology_state.snapshot.as_ref().unwrap().nodes[index].id,
            "c2"
        );
        app.handle_topology_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        assert_eq!(app.topology_state.selected_node_id.as_deref(), Some("c1"));
    }

    #[test]
    fn topology_relationship_arrows_and_pages_reveal_real_records() {
        let mut app = topology_fixture();
        render(&mut app, 80, 24);
        app.activate_target(Target::TopologyPanel(1));
        app.set_focus(Target::TopologyRelationships);
        app.handle_topology_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(app.focus, Target::TopologyRelated(0));
        let last = super::super::topology::related_edge_indices(&app.topology_state).len() - 1;
        for _ in 0..last {
            app.handle_topology_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        }
        assert_eq!(app.focus, Target::TopologyRelated(last));
        assert!(app.topology_state.related_scroll > 0);
        let _ = super::super::ui::focus_order(&app);
        assert!(app
            .layout
            .borrow()
            .entries
            .iter()
            .any(|entry| entry.target == app.focus));
        app.set_focus(Target::TopologyRelationships);
        for _ in 0..20 {
            app.handle_topology_key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE));
        }
        assert!(app.topology_state.related_scroll <= last);
        for _ in 0..20 {
            app.handle_topology_key(KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE));
        }
        assert_eq!(app.topology_state.related_scroll, 0);
        app.topology_state.inspector_scroll = 0;
        app.set_focus(Target::TopologyGraph);
        app.handle_topology_key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE));
        assert_eq!(app.topology_state.inspector_scroll, 0);
    }

    #[test]
    fn topology_tabs_keep_focus_separate_and_refresh_pointer_geometry_before_paint() {
        let mut app = topology_fixture();
        render(&mut app, 80, 24);
        let node_rect = app
            .layout
            .borrow()
            .entries
            .iter()
            .find(|entry| matches!(entry.target, Target::TopologyNode(_)))
            .unwrap()
            .rect;
        app.set_focus(Target::TopologyPanel(0));
        app.handle_topology_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        assert_eq!(app.focus, Target::TopologyPanel(1));
        assert_eq!(app.topology_state.active_panel_tab, 0);
        app.handle_topology_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.topology_state.active_panel_tab, 1);
        assert!(!matches!(
            super::super::ui::hit_test(&app, node_rect.x, node_rect.y),
            Some(Target::TopologyNode(_))
        ));
        app.set_focus(Target::TopologyInspectorTab(0));
        app.handle_topology_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        assert_eq!(app.topology_state.inspector_tab, 0);
        app.handle_topology_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.topology_state.inspector_tab, 1);
        assert_eq!(app.topology_state.related_sel, 0);
        render(&mut app, 160, 50);
        app.activate_target(Target::TopologyPanel(1));
        assert_eq!(app.focus, Target::TopologyInspector);
    }

    #[test]
    fn topology_endpoint_history_preserves_edge_and_original_focus() {
        let mut app = topology_fixture();
        render(&mut app, 160, 50);
        let original = app.topology_state.snapshot.as_ref().unwrap().clone();
        app.activate_target(Target::TopologyEdge(0));
        app.topology_state.active_panel_tab = 1;
        app.topology_state.inspector_scroll = 4;
        // The presentation fixture IDs need not exist in an independent store.
        // Holding a background read lets the history restore be observed first.
        app.topology_reading = true;
        app.activate_target(Target::TopologyEndpoint(1));
        let saved = app.topology_state.history.last().unwrap();
        assert_eq!(saved.selected_edge_id.as_deref(), Some("edge-0"));
        assert_eq!(saved.selected_node_id.as_deref(), Some("c1"));
        assert!(matches!(
            saved.focus,
            Some(super::super::topology::TopologyFocus::Control(
                Target::TopologyEndpoint(1)
            ))
        ));
        app.activate_button(ButtonId::TopologyBack);
        assert_eq!(
            app.topology_state.selected_edge_id.as_deref(),
            Some("edge-0")
        );
        assert_eq!(app.topology_state.active_panel_tab, 1);
        assert_eq!(app.topology_state.inspector_scroll, 4);
        assert!(matches!(
            app.topology_state.pending_focus,
            Some(super::super::topology::TopologyFocus::Control(
                Target::TopologyEndpoint(1)
            ))
        ));
        app.on_work_event(WorkEvent::TopologyLoaded {
            generation: app.topology_generation,
            seed: argos_osint_core::brain_topology::TopologySeed::Claim("c1".into()),
            outcome: Ok(Box::new(original)),
        });
        assert_eq!(app.focus, Target::TopologyEndpoint(1));
        assert_eq!(
            app.topology_state.selected_edge_id.as_deref(),
            Some("edge-0")
        );
    }

    #[test]
    fn topology_refresh_rebinds_stable_focus_and_preserves_other_app_status() {
        use argos_osint_core::brain_topology::TopologySeed;
        for edge_focus in [false, true] {
            let mut app = topology_fixture();
            render(&mut app, 160, 50);
            app.activate_target(if edge_focus {
                Target::TopologyEdge(0)
            } else {
                Target::TopologyNode(1)
            });
            let mut snapshot = app.topology_state.snapshot.as_ref().unwrap().clone();
            snapshot.nodes.reverse();
            snapshot.edges.reverse();
            app.on_work_event(WorkEvent::TopologyLoaded {
                generation: app.topology_generation,
                seed: TopologySeed::Claim("c1".into()),
                outcome: Ok(Box::new(snapshot)),
            });
            let snapshot = app.topology_state.snapshot.as_ref().unwrap();
            match app.focus {
                Target::TopologyNode(index) => assert_eq!(snapshot.nodes[index].id, "a1"),
                Target::TopologyEdge(index) => assert_eq!(snapshot.edges[index].id, "edge-0"),
                _ => panic!("stable graph focus required"),
            }
            let snapshot = snapshot.clone();
            app.module = Some(ModuleId::Intel);
            app.status = "Intel retained status".into();
            app.on_work_event(WorkEvent::TopologyLoaded {
                generation: app.topology_generation,
                seed: TopologySeed::Claim("c1".into()),
                outcome: Ok(Box::new(snapshot)),
            });
            assert_eq!(app.status, "Intel retained status");
        }
    }

    #[test]
    fn topology_rejects_obsolete_same_seed_request_options() {
        use argos_osint_core::brain_topology::{TopologySeed, TopologySnapshot};
        let mut app = app();
        app.module = Some(ModuleId::Brain);
        let claim = app
            .store
            .add_user_claim(
                "Retained exact seed",
                "fact",
                MemorySource {
                    app: "test".into(),
                    conversation_id: "topology-race".into(),
                    message_id: None,
                    reference: None,
                },
            )
            .unwrap();
        let seed = TopologySeed::Claim(claim.id.clone());
        app.open_topology(seed.clone());
        let obsolete = app.topology_generation;
        app.topology_state.depth = 3;
        app.refresh_topology();
        app.on_work_event(WorkEvent::TopologyLoaded {
            generation: obsolete,
            seed,
            outcome: Ok(Box::new(TopologySnapshot::default())),
        });
        assert_eq!(
            app.topology_state.snapshot.as_ref().unwrap().root_id,
            claim.id
        );
        assert_eq!(app.topology_state.depth, 3);
    }

    #[test]
    fn topology_entries_recenter_and_generation_are_scoped() {
        use argos_osint_core::brain_topology::{TopologySeed, TopologySnapshot};
        let mut app = app();
        app.module = Some(ModuleId::Brain);
        let claim = app
            .store
            .add_user_claim(
                "Exact isolated claim",
                "fact",
                MemorySource {
                    app: "test".into(),
                    conversation_id: "topology".into(),
                    message_id: None,
                    reference: None,
                },
            )
            .unwrap();
        assert!(app.open_memory_detail(&claim.id));
        app.scrolls.path = 7;
        app.activate_target(Target::DetailPath);
        assert_eq!(app.brain_list_mode, BrainListMode::Topology);
        let snap = app.topology_state.snapshot.as_ref().unwrap();
        assert_eq!(snap.root_id, claim.id);
        assert_eq!(snap.nodes.len(), 1);
        assert!(snap.edges.is_empty());
        let generation = app.topology_generation;
        let seed = TopologySeed::Claim(claim.id.clone());
        app.topology_generation += 1;
        app.module = Some(ModuleId::Intel); // obsolete completion cannot steal another app's status
        app.status = "Intel context".into();
        app.on_work_event(WorkEvent::TopologyLoaded {
            generation,
            seed,
            outcome: Ok(Box::new(TopologySnapshot::default())),
        });
        assert_eq!(
            app.topology_state.snapshot.as_ref().unwrap().root_id,
            claim.id
        );
        assert_eq!(app.status, "Intel context");
        app.module = Some(ModuleId::Brain);
        app.activate_button(ButtonId::TopologyBack);
        assert_eq!(app.brain_list_mode, BrainListMode::Graph);
        assert_eq!(app.scrolls.path, 7);
    }

    #[test]
    fn topology_intel_metadata_actor_entry_restores_brief() {
        let mut app = app();
        app.module = Some(ModuleId::Intel);
        app.intel_page = IntelPage::Briefing;
        app.intel_articles = vec![intel_brief_image_article("topology-article", "")];
        app.intel_actors = vec![argos_osint_core::atlas_metadata::ArticleActor {
            id: "source-local-actor".into(),
            display_name: "Unindexed actor".into(),
            type_hint: "organization".into(),
            spellings: vec![],
            mentions: vec![],
        }];
        app.scrolls.intel_extracted = 12;
        app.focus = Target::IntelTopologyActor(0);
        app.activate_target(Target::IntelTopologyActor(0));
        assert_eq!(app.brain_list_mode, BrainListMode::Topology);
        assert!(matches!(
            app.topology_state.seed,
            Some(argos_osint_core::brain_topology::TopologySeed::RetainedActor { .. })
        ));
        app.activate_button(ButtonId::TopologyBack);
        assert_eq!(app.module, Some(ModuleId::Intel));
        assert_eq!(app.intel_page, IntelPage::Briefing);
        assert_eq!(app.scrolls.intel_extracted, 12);
        assert_eq!(app.focus, Target::IntelTopologyActor(0));
    }

    #[test]
    fn topology_back_checks_its_owner_and_handles_deleted_origin() {
        let mut app = app();
        app.module = Some(ModuleId::Brain);
        app.intel_articles = vec![intel_brief_image_article("unrelated", "")];
        let claim = app
            .store
            .add_user_claim(
                "Origin claim",
                "fact",
                MemorySource {
                    app: "test".into(),
                    conversation_id: "origin".into(),
                    message_id: None,
                    reference: None,
                },
            )
            .unwrap();
        assert!(app.open_memory_detail(&claim.id));
        app.activate_target(Target::DetailPath);
        app.intel_articles.clear();
        app.activate_button(ButtonId::TopologyBack);
        assert_eq!(app.brain_list_mode, BrainListMode::Graph);
        assert!(!app.status.contains("Origin article"));
        app.activate_target(Target::DetailPath);
        app.store.delete_memory(&claim.id).unwrap();
        app.activate_button(ButtonId::TopologyBack);
        assert_eq!(app.brain_list_mode, BrainListMode::List);
        assert!(app.brain_detail.memory.is_none());
        assert!(app.status.contains("deleted"));

        app.module = Some(ModuleId::Intel);
        app.intel_page = IntelPage::Briefing;
        app.intel_articles = vec![intel_brief_image_article("origin-article", "")];
        app.open_topology(argos_osint_core::brain_topology::TopologySeed::Claim(
            "missing".into(),
        ));
        app.intel_articles.clear();
        app.activate_button(ButtonId::TopologyBack);
        assert_eq!(app.intel_page, IntelPage::Bulletin);
        assert!(app.status.contains("Origin article changed"));
    }

    #[test]
    fn dump_brain_topology_screens() {
        let mut initial_app = topology_fixture();
        let text = buffer_text(&render(&mut initial_app, 160, 50));
        assert!(text.contains("Topology mapper"));
        assert!(text.contains("Harbor contract"));
        assert!(!text.contains("Northstar Labs"));
        let Ok(dir) = std::env::var("ARGOS_SCREEN_DIR") else {
            return;
        };
        std::fs::create_dir_all(&dir).unwrap();
        for (width, height) in [
            (160, 50),
            (132, 40),
            (131, 40),
            (120, 40),
            (100, 40),
            (100, 34),
            (100, 33),
            (100, 32),
            (80, 24),
            (60, 20),
            (60, 18),
            (40, 20),
        ] {
            let mut app = topology_fixture();
            dump_cells(
                &mut app,
                &dir,
                &format!("topology-graph-{width}x{height}"),
                width,
                height,
            );
            app.topology_state.active_panel_tab = 1;
            app.topology_state.selected_edge_id = Some("edge-5".into());
            dump_cells(
                &mut app,
                &dir,
                &format!("topology-edge-{width}x{height}"),
                width,
                height,
            );
            app.topology_state.active_panel_tab = 2;
            dump_cells(
                &mut app,
                &dir,
                &format!("topology-controls-{width}x{height}"),
                width,
                height,
            );
            app.topology_state.snapshot = None;
            app.topology_loading = true;
            app.topology_state.active_panel_tab = 0;
            dump_cells(
                &mut app,
                &dir,
                &format!("topology-loading-{width}x{height}"),
                width,
                height,
            );
        }
        for (width, height) in [(160, 50), (80, 24), (60, 18)] {
            let mut isolated = topology_fixture();
            let snapshot = isolated.topology_state.snapshot.as_mut().unwrap();
            snapshot.nodes.retain(|node| node.is_root);
            snapshot.edges.clear();
            snapshot.total_nodes = 1;
            snapshot.total_edges = 0;
            snapshot.discovered_nodes = 1;
            snapshot.discovered_edges = 0;
            dump_cells(
                &mut isolated,
                &dir,
                &format!("topology-isolated-{width}x{height}"),
                width,
                height,
            );
            let mut stale = topology_fixture();
            stale.topology_error =
                Some("Read failed · previous snapshot retained · r retries".into());
            dump_cells(
                &mut stale,
                &dir,
                &format!("topology-stale-{width}x{height}"),
                width,
                height,
            );
            stale.topology_state.snapshot = None;
            stale.topology_error = Some("Read failed · snapshot unavailable · r retries".into());
            dump_cells(
                &mut stale,
                &dir,
                &format!("topology-error-{width}x{height}"),
                width,
                height,
            );
            let mut crowded = topology_fixture();
            let snapshot = crowded.topology_state.snapshot.as_mut().unwrap();
            for index in 0..24 {
                let mut node = snapshot.nodes[1].clone();
                node.id = format!("crowded-{index}");
                node.label = format!("東京 Freight operator {index}");
                let mut edge = snapshot.edges[0].clone();
                edge.id = format!("crowded-edge-{index}");
                edge.to = node.id.clone();
                snapshot.nodes.push(node);
                snapshot.edges.push(edge);
            }
            snapshot.total_nodes = snapshot.nodes.len();
            snapshot.total_edges = snapshot.edges.len();
            snapshot.discovered_nodes = snapshot.nodes.len();
            snapshot.discovered_edges = snapshot.edges.len();
            dump_cells(
                &mut crowded,
                &dir,
                &format!("topology-crowded-{width}x{height}"),
                width,
                height,
            );
        }
    }
}
