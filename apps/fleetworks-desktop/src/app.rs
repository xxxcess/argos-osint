//! Shared production state and shell; all boundary results are revision fenced.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use fleetworks_contracts::ipc::IpcEvent;
use fleetworks_contracts::ipc::ShutdownAck;
use fleetworks_contracts::readiness::ReadinessCheckId;
use fleetworks_contracts::readiness::ReadinessState;
use fleetworks_contracts::readiness::WorkstationReadiness;
use fleetworks_contracts::search::ActorSearchQuery;
use fleetworks_contracts::Client;
use fleetworks_workstation::native::canonical_local_origin;
use fleetworks_workstation::native::BackendKind;
use fleetworks_workstation::native::DEFAULT_LOCAL_ORIGIN;
use fleetworks_workstation::native::LOCAL_BACKEND_DIR;
use rune::AppSpec;
use rune::Cx;
use rune::El;
use rune::Key;
use rune::KeyEvent;
use rune::Live;
use rune::Spring;
use rune::SpringPair;
use rune::Transition;
use rune::Tx;
use rune_kit::shell;
use rune_kit::shell::PaneState;
use rune_kit::shell::RowStatus;
use rune_kit::shell::Sidebar;
use rune_kit::shell::SidebarHeader;
use rune_kit::shell::SidebarRow;
use rune_kit::shell::SplitView;
use rune_kit::shell::Toolbar;
use rune_kit::shell::TOOLBAR_HEIGHT;
use rune_kit::Button;
use rune_kit::ButtonVariant;
use rune_kit::ControlSize;
use rune_kit::IconButton;
use rune_kit::IconButtonVariant;

use crate::accessibility::Accessibility;
use crate::accessibility::ViewPreferences;
use crate::backend::Backend;
use crate::backend::ReadKind;
use crate::brain::Brain;
use crate::classifier_activation::ActivationAction;
use crate::classifier_activation::ClassifierActivation;
use crate::configuration::Configuration;
use crate::configuration::ConfigurationAction;
use crate::connection::Connected;
use crate::connection::Connection;
use crate::connection::ConnectionStatus;
use crate::connection::GatewayEffects;
use crate::connection::RequestTask;
use crate::connection_profiles::ConnectionProfiles;
use crate::connection_profiles::ProfileAction;
use crate::icons;
use crate::job_actions::JobAction;
use crate::job_actions::JobActions;
use crate::jobs_query::JobsQueryState;
use crate::local_backend::LocalBackendProcess;
use crate::local_backend::LocalBackendState;
use crate::local_backend::LocalJobProgress;
use crate::logs::ActivityTab;
use crate::logs::LogAction;
use crate::logs::LogState;
use crate::maintenance::Maintenance;
use crate::material;
use crate::material::GlassSurface;
use crate::rebind::Rebind;
use crate::registration::Registration;
use crate::submission::SubmissionKind;
use crate::submission::Submissions;
use crate::theme;
use crate::theme::StatusTone;
use crate::transfer::Transfer;
use crate::transfer::TransferAction;
use crate::ui_components;
use crate::views;
use crate::workspaces::WorkspaceCommand;
use crate::workspaces::Workspaces;
use fleetworks_contracts::readiness::DatasetKind;
use fleetworks_contracts::readiness::MaintenanceOperation;

/// All native pages. Backend resources and local activity have distinct ownership.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Page {
    Seed,
    Atlas,
    Brain,
    Tools,
    Models,
    Profile,
    #[default]
    Workstation,
    Workspaces,
    Jobs,
}

impl Page {
    pub const ALL: [Self; 6] = [
        Self::Seed,
        Self::Atlas,
        Self::Brain,
        Self::Tools,
        Self::Models,
        Self::Jobs,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Self::Seed => "DB Seed & Train",
            Self::Atlas => "News Cycles",
            Self::Brain => "Brain Search",
            Self::Tools => "OSINT Tools",
            Self::Models => "Model Assignments",
            Self::Profile => "Profile",
            Self::Workstation => "Workstation Status",
            Self::Workspaces => "Workspaces",
            Self::Jobs => "Jobs & Logs",
        }
    }

    fn compact_title(self) -> &'static str {
        match self {
            Self::Seed => "Seed & Train",
            Self::Atlas => "Cycles",
            Self::Brain => "Brain",
            Self::Tools => "Tools",
            Self::Models => "Models",
            Self::Profile => "Profile",
            Self::Workstation => "Status",
            Self::Workspaces => "Workspaces",
            Self::Jobs => "Jobs",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::Seed => "seed",
            Self::Atlas => "atlas",
            Self::Brain => "brain",
            Self::Tools => "tools",
            Self::Models => "models",
            Self::Profile => "profile",
            Self::Workstation => "workstation",
            Self::Workspaces => "workspaces",
            Self::Jobs => "jobs",
        }
    }

    /// Crisp 16px vector icon matching this destination.
    pub fn icon(self) -> &'static rune::Icon {
        match self {
            Self::Seed => &icons::SEED,
            Self::Atlas => &icons::ATLAS,
            Self::Brain => &icons::BRAIN,
            Self::Tools => &icons::TOOLS,
            Self::Models => &icons::MODELS,
            Self::Profile | Self::Workstation | Self::Workspaces => &icons::WORKSTATION,
            Self::Jobs => &icons::JOBS,
        }
    }

    /// Concise description for the selected destination's context rail.
    pub fn summary_hint(self) -> &'static str {
        match self {
            Self::Seed => "Stage seeds, classifier runs & maintenance",
            Self::Atlas => "Inspect news collection cycles & trigger runs",
            Self::Brain => "Search actors and inspect intelligence dossiers",
            Self::Tools => "Configure API tool providers & credentials",
            Self::Models => "Assign routing models & verify accounts",
            Self::Profile | Self::Workstation | Self::Workspaces => {
                "Inspect host telemetry, registration & identity"
            }
            Self::Jobs => "Monitor live queue execution & workstation logs",
        }
    }

    pub fn is_backend(self) -> bool {
        matches!(
            self,
            Self::Seed | Self::Atlas | Self::Brain | Self::Tools | Self::Models | Self::Profile
        )
    }
}

/// Active progressive disclosure stage on `DB Seed & Train`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SeedStage {
    #[default]
    Seed,
    Classifier,
    Maintenance,
}

/// Active view mode on `Model Assignments`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ModelsTab {
    #[default]
    Roles,
    Providers,
}

/// Active dossier tab on `Brain Search`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum DossierFacet {
    #[default]
    Overview,
    Associations,
}

/// Clicked host telemetry tile modal inspector on `Workstation Status`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkstationInspector {
    Cpu,
    Memory,
    Disk,
}

#[derive(Clone, Debug, Default)]
pub enum Load<T> {
    #[default]
    Idle,
    Pending,
    Loading,
    Ready(T),
    Failed(String),
}

/// Observable boundary intent for deterministic fixtures; contains no credential values.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Intent {
    Connect,
    SwitchBackendKind(BackendKind),
    StopLocalBackend,
    PairingConnect,
    PairingRestart,
    PairingSessionOnly,
    Readiness,
    Search { query: String },
    Dossier { actor_id: String },
    Workspace(WorkspaceCommand),
    Interfaces,
    Register,
    OpenIntel,
    ReconcileRegistration { request_id: String },
    RebindWorkspace { name: String, ip: String },
    ReconcileRebind { request_id: String },
    BackendRead { kind: ReadKind },
    ChooseDataset { kind: DatasetKind },
    DownloadGithubDataset { kind: DatasetKind },
    UploadDataset { kind: DatasetKind },
    Datasets,
    Maintenance { operation: MaintenanceOperation },
    MaintenanceJobStatus { job_id: String },
    Configuration { action: ConfigurationAction },
    PrepareProfileImport,
    ProfileTransfer { action: TransferAction },
    CopyWorkspacePath { name: String },
    SubmitInteractive { kind: SubmissionKind },
    PostInteractive { kind: SubmissionKind },
    VerifyInteractive { kind: SubmissionKind },
    InteractiveToolResult,
    ResolveInteractive { kind: SubmissionKind },
    PrepareJobAction { action: JobAction },
    PostJobAction { action: JobAction },
    ClassifierActivation { action: ActivationAction },
    ConnectionProfile { action: ProfileAction },
    ConnectionCredentialSelected,
    ChooseConnectionCredential,
    OperationalLogs { action: LogAction },
    CopyLogSummaries,
    SaveLogExport { partial: bool },
}

/// One state owns navigation, context, client and asynchronous request guards.
pub struct State {
    pub accessibility: Accessibility,
    /// Cold-launch splash and connection choice. Finished by default so fixtures skip it.
    pub startup: crate::startup::Startup,
    /// Contextual Assistant dock. It sends nothing until a typed chat capability exists.
    pub assistant: crate::assistant::Assistant,
    /// Fixture only: draw the component catalog in place of the page.
    pub(crate) fixture_catalog: bool,
    pub(crate) fixture_bundled_csv: bool,
    /// Observed job, readiness and connection transitions for this authority.
    pub inbox: crate::inbox::Inbox,
    pub(crate) inbox_qa: crate::inbox_qa::InboxQa,
    /// Cmd/Ctrl+K command panel over a fixed list of known actions.
    pub commands: crate::command_panel::CommandPanel,
    pub page: Page,
    pub setup_gateway: bool,
    pub backend_kind: BackendKind,
    pub local_backend: LocalBackendState,
    pub(crate) settings_open: bool,
    pub(crate) settings_section: crate::workstation_settings::SettingsSection,
    pub tool_key_dialog: bool,
    pub setup_advanced_open: bool,
    pub workstation_inspector: Option<WorkstationInspector>,
    pub(crate) workstation_diagnostics_open: bool,
    pub workstation_status_checks_open: bool,
    pub seed_stage: SeedStage,
    pub models_tab: ModelsTab,
    pub models_provider_dialog: bool,
    pub models_role_dialog: bool,
    pub brain_facet: DossierFacet,
    pub selected_readiness: Option<String>,
    pub(crate) expanded_readiness: BTreeSet<ReadinessCheckId>,
    pub selected_cycle_id: Option<String>,
    pub selected_job_id: Option<String>,
    pub confirm_cycle_delete: bool,
    pub confirm_maintenance_dialog: bool,
    pub pointer_offset: Live<[f32; 2]>,
    pub shadow_offset: Live<[f32; 2]>,
    pub cycles: crate::workstation_dashboard::Cycles,
    pub host_metrics: Load<crate::workstation_dashboard::HostMetrics>,
    pub(crate) host_metrics_request: Option<RequestTask>,
    /// UI clock reading when `host_metrics` last succeeded.
    pub(crate) host_metrics_observed_at: Option<f64>,
    /// Set when a refresh failed after a success. Cleared by the next success.
    pub(crate) host_metrics_error: Option<String>,
    pub sidebar: PaneState,
    pub connection: Connection,
    pub pairing: crate::pairing::Pairing,
    pub connection_profiles: ConnectionProfiles,
    pub brain: Brain,
    pub backend: Backend,
    pub maintenance: Maintenance,
    pub configuration: Configuration,
    pub transfer: Transfer,
    pub submissions: Submissions,
    pub job_actions: JobActions,
    pub classifier_activation: ClassifierActivation,
    pub scoped_logs: LogState,
    pub scoped_jobs: JobsQueryState,
    pub activity_tab: ActivityTab,
    pub readiness: Load<WorkstationReadiness>,
    pub readiness_received_at: Option<f64>,
    /// The last refresh failed while earlier same-scope checks are still shown as stale.
    pub readiness_error: Option<String>,
    pub(crate) clock: f64,
    pub(crate) reveal: Option<String>,
    pub selected_workspace: Option<PathBuf>,
    pub workspaces: Workspaces,
    pub registration: Registration,
    pub rebind: Rebind,
    pub(crate) origin_revision: u64,
    pub fixture: bool,
    pub intents: Vec<Intent>,
    /// Injected only by developer/test callers; the production executable has no fixture flag.
    pub fixture_effect: Option<fn(&mut State, &Intent)>,
    pub(crate) services: Option<Arc<GatewayEffects>>,
    pub(crate) client: Option<Client>,
    connection_request: Option<RequestTask>,
    local_shutdown_request: Option<RequestTask>,
    readiness_request: Option<RequestTask>,
    pub(crate) search_request: Option<RequestTask>,
    pub(crate) dossier_request: Option<RequestTask>,
    pub(crate) readiness_revision: u64,
    pub(crate) readiness_refresh_at: f64,
    pub(crate) maintenance_jobs_refresh_at: f64,
    pub(crate) jobs_refresh_at: f64,
    pub(crate) jobs_discovery_at: f64,
    pub(crate) host_metrics_revision: u64,
    pub(crate) host_metrics_refresh_at: f64,
    pub(crate) atlas_refresh_at: f64,
}

impl Default for State {
    fn default() -> Self {
        Self {
            accessibility: Accessibility::default(),
            startup: crate::startup::Startup::default(),
            assistant: crate::assistant::Assistant::default(),
            fixture_catalog: false,
            inbox: crate::inbox::Inbox::default(),
            inbox_qa: crate::inbox_qa::InboxQa::default(),
            commands: crate::command_panel::CommandPanel::default(),
            page: Page::Workstation,
            setup_gateway: false,
            backend_kind: BackendKind::Remote,
            local_backend: LocalBackendState::default(),
            settings_open: false,
            settings_section: Default::default(),
            tool_key_dialog: false,
            setup_advanced_open: false,
            workstation_inspector: None,
            workstation_diagnostics_open: false,
            workstation_status_checks_open: false,
            seed_stage: SeedStage::Seed,
            models_tab: ModelsTab::Roles,
            models_provider_dialog: false,
            models_role_dialog: false,
            brain_facet: DossierFacet::Overview,
            selected_readiness: None,
            expanded_readiness: BTreeSet::new(),
            selected_cycle_id: None,
            selected_job_id: None,
            confirm_cycle_delete: false,
            confirm_maintenance_dialog: false,
            pointer_offset: Live::new([0.0, 0.0]),
            shadow_offset: Live::new([0.0, 0.0]),
            cycles: crate::workstation_dashboard::Cycles::default(),
            host_metrics: Load::Idle,
            host_metrics_request: None,
            host_metrics_observed_at: None,
            host_metrics_error: None,
            sidebar: PaneState::new(188.0).range(176.0, 340.0),
            connection: Connection::default(),
            pairing: crate::pairing::Pairing::default(),
            connection_profiles: ConnectionProfiles::initial(),
            brain: Brain::default(),
            backend: Backend::default(),
            maintenance: Maintenance::default(),
            configuration: Configuration::default(),
            transfer: Transfer::default(),
            submissions: Submissions::default(),
            job_actions: JobActions::default(),
            classifier_activation: ClassifierActivation::default(),
            scoped_logs: LogState::default(),
            scoped_jobs: JobsQueryState::default(),
            activity_tab: ActivityTab::Jobs,
            readiness: Load::Idle,
            readiness_received_at: None,
            readiness_error: None,
            clock: 0.0,
            reveal: None,
            selected_workspace: None,
            workspaces: Workspaces::default(),
            registration: Registration::default(),
            rebind: Rebind::default(),
            origin_revision: 0,
            fixture: false,
            fixture_bundled_csv: false,
            intents: Vec::new(),
            fixture_effect: None,
            services: None,
            client: None,
            connection_request: None,
            local_shutdown_request: None,
            readiness_request: None,
            search_request: None,
            dossier_request: None,
            readiness_revision: 0,
            readiness_refresh_at: 0.0,
            maintenance_jobs_refresh_at: 0.0,
            jobs_refresh_at: 0.0,
            jobs_discovery_at: 0.0,
            host_metrics_revision: 0,
            host_metrics_refresh_at: 0.0,
            atlas_refresh_at: 0.0,
        }
    }
}

impl State {
    /// Local confirmation fence, separate from every server context revision.
    pub fn origin_revision(&self) -> u64 {
        self.origin_revision
    }

    pub(crate) fn reveal_control(&mut self, key: &str) {
        self.reveal = Some(key.to_owned());
    }
    /// An effects-disabled state for development/test views, never a real user home.
    pub fn fixture() -> Self {
        Self {
            fixture: true,
            setup_gateway: false,
            ..Self::default()
        }
    }

    /// Use synthetic packaged resource paths for the startup fixture.
    pub fn seed_startup_bundled_dataset_fixture(&mut self) {
        if self.fixture {
            self.fixture_bundled_csv = true;
        }
    }

    pub fn with_services(services: Arc<GatewayEffects>) -> Self {
        let mut state = Self {
            services: Some(services),
            ..Self::default()
        };
        state.backend.article_day =
            rune::TextEdit::new(chrono::Utc::now().format("%Y-%m-%d").to_string());
        state
    }

    pub fn navigate(&mut self, page: Page) {
        if self.setup_gateway && self.pairing.busy() {
            self.disconnect();
        }
        self.setup_gateway = false;
        self.settings_open = false;
        if self.tool_key_dialog {
            self.tool_key_dialog = false;
            self.clear_tool_credential_draft();
            self.configuration_edited();
        }
        self.workstation_inspector = None;
        self.models_provider_dialog = false;
        self.models_role_dialog = false;
        self.selected_readiness = None;
        self.selected_cycle_id = None;
        self.selected_job_id = None;
        self.confirm_cycle_delete = false;
        self.confirm_maintenance_dialog = false;
        self.page = if self.connected() || page == Page::Workstation {
            page
        } else {
            Page::Workstation
        };
        self.sync_jobs_binding();
        self.sync_log_visibility();
        if self.connected() {
            self.refresh_page();
        }
    }

    /// Drop ephemeral reads only. Accepted backend work has an independent lifetime.
    pub fn cancel_requests(&mut self) {
        self.inbox_qa.detach();
        self.pairing.cancel();
        self.connection_request = None;
        self.host_metrics_request = None;
        self.host_metrics_revision = self.host_metrics_revision.wrapping_add(1);
        self.cycles.invalidate();
        self.connection_profiles.cancel_tests();
        self.readiness_request = None;
        self.search_request = None;
        self.dossier_request = None;
        self.registration.request = None;
        self.registration.interface_request = None;
        self.rebind.request = None;
        self.workspaces.request = None;
        self.workspaces.busy = false;
        self.workspaces.revision = self.workspaces.revision.wrapping_add(1);
        self.backend.cancel_reads();
        self.maintenance.cancel_requests();
        self.configuration.request = None;
        self.transfer.request = None;
        self.submissions.cancel_requests();
        self.job_actions.request = None;
        self.classifier_activation.cancel_requests();
        self.scoped_logs.cancel_requests();
        self.scoped_jobs.cancel_reads();
        self.brain.in_flight = false;
    }

    /// Wipe ephemeral RAM-only session credentials without touching durable keyring entries.
    pub(crate) fn clear_session_secrets(&mut self) {
        self.pairing.storage_mode =
            fleetworks_workstation::native::CredentialStorage::SystemKeyring;
        if let Some(services) = &self.services {
            services.clear_session_secrets();
        }
    }

    pub(crate) fn close_window_cleanup(&mut self) {
        self.cancel_requests();
        self.clear_session_secrets();
        // Window close preserves any active local companion backend and running jobs.
    }

    pub(crate) fn quit_cleanup(&mut self) {
        self.shutdown_local_backend("application_quit", true);
        self.cancel_requests();
        self.clear_session_secrets();
    }

    /// Whether the workstation is configured for or attached to the local embedded backend.
    #[must_use]
    pub fn is_local_backend(&self) -> bool {
        self.backend_kind == BackendKind::Local
    }

    /// Expected or provisioned private `0o700` state directory label for local display.
    #[must_use]
    pub fn local_state_dir_label(&self) -> String {
        if let Some(dir) = &self.local_backend.state_dir {
            return dir.display().to_string();
        }
        if let Some(container) = &self.workspaces.container {
            let slug = self.local_profile_slug();
            return container
                .root()
                .join(LOCAL_BACKEND_DIR)
                .join(slug)
                .display()
                .to_string();
        }
        format!(
            "<workstation>/{LOCAL_BACKEND_DIR}/{}",
            self.local_profile_slug()
        )
    }

    pub(crate) fn local_profile_slug(&self) -> String {
        canonical_local_origin(&self.connection.origin.text)
            .ok()
            .and_then(|origin| origin.strip_prefix("local://").map(str::to_owned))
            .unwrap_or_else(|| "embedded".to_owned())
    }

    /// Switch between Distributed and Embedded without carrying credentials or URLs across.
    pub fn set_backend_kind(&mut self, kind: BackendKind) {
        if self.backend_kind == kind {
            return;
        }
        self.disconnect();
        self.backend_kind = kind;
        self.record(Intent::SwitchBackendKind(kind));
        match kind {
            BackendKind::Local => {
                if canonical_local_origin(&self.connection.origin.text).is_err() {
                    self.connection.origin = rune::TextEdit::new(DEFAULT_LOCAL_ORIGIN);
                }
                self.connection.token_file = rune::TextEdit::new("");
                self.connection_profiles.credential = None;
                self.connection_profiles.operator_token_override = false;
                if !self.fixture {
                    self.local_backend.refresh_binary_status();
                }
            }
            BackendKind::Remote => {
                if canonical_local_origin(&self.connection.origin.text).is_ok() {
                    self.connection.origin =
                        rune::TextEdit::new(fleetworks_contracts::DEFAULT_API_ORIGIN);
                }
                self.local_backend.clear_runtime_notices();
            }
            _ => {}
        }
    }

    /// Explicitly stop the running local embedded companion backend cleanly.
    pub fn stop_local_backend(&mut self) {
        self.record(Intent::StopLocalBackend);
        self.disconnect();
    }

    pub(crate) fn shutdown_local_backend(&mut self, reason: &str, wait_for_quit: bool) {
        self.local_backend.event_task = None;
        let Some(process) = self.local_backend.session.take() else {
            return;
        };
        self.client = None;
        self.local_backend.stopping = true;
        self.local_backend.shutdown_message =
            Some(format!("Stopping the embedded backend ({reason})…"));
        if let Some(services) = &self.services {
            self.local_shutdown_request =
                services.shutdown_local_backend(process, reason.to_owned(), wait_for_quit);
        }
    }

    /// Isolate data immediately on destination/auth changes and reject old callbacks.
    pub fn disconnect(&mut self) {
        self.shutdown_local_backend("disconnect", false);
        self.clear_session_secrets();
        self.reset_connection_state();
    }

    /// Reset transient connection state without wiping RAM-only pending pairing secrets.
    pub(crate) fn disconnect_keeping_session_secrets(&mut self) {
        self.reset_connection_state();
    }

    fn reset_connection_state(&mut self) {
        self.cancel_requests();
        self.activity_tab = ActivityTab::Jobs;
        self.startup.data_step = None;
        self.startup.data_checks = false;
        self.startup.data_reset = [false; 3];
        self.workstation_diagnostics_open = false;
        self.workstation_status_checks_open = false;
        self.setup_advanced_open = false;
        self.tool_key_dialog = false;
        self.expanded_readiness.clear();
        self.selected_readiness = None;
        self.workstation_inspector = None;
        self.invalidate_origin();
        self.registration.interfaces = Load::Idle;
        self.registration.selected_interface = None;
        self.connection.epoch = self.connection.epoch.wrapping_add(1);
        self.client = None;
        self.connection.ready = None;
        self.connection.capabilities = None;
        self.connection.status = ConnectionStatus::Disconnected;
        self.page = Page::Workstation;
        self.readiness = Load::Idle;
        self.readiness_received_at = None;
        self.readiness_error = None;
        self.readiness_refresh_at = 0.0;
        self.maintenance_jobs_refresh_at = 0.0;
        self.jobs_refresh_at = 0.0;
        self.jobs_discovery_at = 0.0;
        self.host_metrics = Load::Idle;
        self.host_metrics_observed_at = None;
        self.host_metrics_error = None;
        self.host_metrics_refresh_at = 0.0;
        self.atlas_refresh_at = 0.0;
        self.brain = Brain::default();
        self.backend = Backend::default();
        self.backend.article_day =
            rune::TextEdit::new(chrono::Utc::now().format("%Y-%m-%d").to_string());
        self.maintenance = Maintenance::default();
        self.configuration = Configuration::default();
        self.transfer = Transfer::default();
        self.submissions = Submissions::default();
        self.job_actions = JobActions::default();
        self.classifier_activation = ClassifierActivation::default();
        self.rebind = Rebind::default();
    }

    pub fn connect(&mut self) {
        if self.backend_kind == BackendKind::Local {
            self.connect_local();
            return;
        }
        if self.uses_native_pairing() {
            self.connect_pairing(false);
            return;
        }
        self.connection_profiles.initial_selection_applied = true;
        self.disconnect();
        self.connection.status = ConnectionStatus::Connecting;
        self.record(Intent::Connect);
        if let Some(services) = &self.services {
            self.connection_request = services.connect(
                self.connection.origin.text.clone(),
                self.editor_credential(),
                self.connection.epoch,
            );
        }
    }

    /// Start or attach to the supervised local embedded companion backend over framed stdio IPC.
    pub fn connect_local(&mut self) {
        self.backend_kind = BackendKind::Local;
        self.connection_profiles.initial_selection_applied = true;
        self.disconnect();
        self.local_backend.clear_runtime_notices();
        if canonical_local_origin(&self.connection.origin.text).is_err() {
            self.connection.origin = rune::TextEdit::new(DEFAULT_LOCAL_ORIGIN);
        }
        if !self.fixture {
            self.local_backend.refresh_binary_status();
        }
        self.connection.status = ConnectionStatus::Connecting;
        self.record(Intent::Connect);
        if let Some(services) = &self.services {
            let slug = self.local_profile_slug();
            let explicit_dir = self.local_backend.state_dir.clone();
            self.connection_request = services.connect_local(
                self.local_backend.cli_binary_override.clone(),
                self.workspaces.container.clone(),
                explicit_dir,
                slug,
                self.connection.epoch,
            );
        }
    }

    pub(crate) fn complete_connection(&mut self, epoch: u64, result: Result<Connected, String>) {
        if self.connection.epoch != epoch {
            return;
        }
        self.connection_request = None;
        match result {
            Ok(connected) => {
                self.connection_profiles.credential = connected.credential;
                self.connection_profiles.editing = false;
                self.client = Some(connected.client);
                self.connection.ready = Some(connected.ready);
                self.connection.capabilities = Some(connected.capabilities);
                self.connection.status = ConnectionStatus::Connected;
                if let Some(process) = connected.local_backend {
                    self.local_backend.state_dir = Some(process.state_dir().to_path_buf());
                    self.local_backend.session = Some(Arc::clone(&process));
                    self.local_backend.stopping = false;
                    self.local_backend.clear_runtime_notices();
                    if let Some(services) = &self.services {
                        self.local_backend.event_task =
                            services.watch_local_backend_events(process, epoch);
                    }
                }
                self.ensure_registration();
                self.refresh_readiness();
                if self.page == Page::Workstation {
                    self.host_metrics_refresh_at = self.clock + 5.0;
                    self.refresh_host_metrics();
                }
            }
            Err(error) => self.connection.status = ConnectionStatus::Failed(error),
        }
    }

    pub(crate) fn apply_local_backend_event(
        &mut self,
        epoch: u64,
        process: &Arc<LocalBackendProcess>,
        event: IpcEvent,
    ) {
        if self.connection.epoch != epoch
            || !self
                .local_backend
                .session
                .as_ref()
                .is_some_and(|active| Arc::ptr_eq(active, process))
        {
            return;
        }
        match event {
            IpcEvent::JobProgress {
                job_id,
                phase,
                summary,
            } => {
                self.local_backend.last_job_progress = Some(LocalJobProgress {
                    job_id,
                    phase,
                    summary,
                });
            }
            IpcEvent::WorkstationReadinessChanged if self.connected() => {
                self.refresh_readiness();
            }
            IpcEvent::PressurePaused { reason_code } => {
                self.local_backend.pressure_paused = Some(reason_code);
            }
            IpcEvent::ShutdownNotice {
                checkpointed,
                relinquished_leases,
            } => {
                self.local_backend.shutdown_message = Some(format!(
                    "Embedded backend shutdown notice (checkpointed={checkpointed}, relinquished_leases={relinquished_leases})."
                ));
            }
            _ => {}
        }
    }

    pub(crate) fn handle_local_backend_lag(
        &mut self,
        epoch: u64,
        process: &Arc<LocalBackendProcess>,
        skipped: u64,
    ) {
        if self.connection.epoch != epoch
            || !self
                .local_backend
                .session
                .as_ref()
                .is_some_and(|active| Arc::ptr_eq(active, process))
        {
            return;
        }
        self.local_backend.shutdown_message = Some(format!(
            "Embedded IPC event stream skipped {skipped} notifications; refreshing readiness."
        ));
        if self.connected() {
            self.refresh_readiness();
        }
    }

    pub(crate) fn handle_local_backend_closed(
        &mut self,
        epoch: u64,
        _closed: bool,
        stderr: String,
    ) {
        if self.connection.epoch != epoch {
            return;
        }
        if !stderr.is_empty() {
            self.local_backend.stderr_excerpt = Some(stderr.clone());
        }
        if !self.local_backend.stopping && self.local_backend.session.is_some() {
            self.local_backend.session = None;
            self.local_backend.event_task = None;
            self.client = None;
            let detail = if stderr.is_empty() {
                "Embedded backend IPC pipe closed unexpectedly (HTTP fallback disabled).".to_owned()
            } else {
                format!(
                    "Embedded backend IPC pipe closed unexpectedly (HTTP fallback disabled): {stderr}"
                )
            };
            self.local_backend.pipe_closed_error = Some(detail.clone());
            self.connection.status = ConnectionStatus::Failed(detail);
        }
    }

    pub(crate) fn complete_local_backend_shutdown(
        &mut self,
        outcome: Result<ShutdownAck, String>,
        stderr: String,
    ) {
        self.local_backend.stopping = false;
        self.local_shutdown_request = None;
        if !stderr.is_empty() {
            self.local_backend.stderr_excerpt = Some(stderr);
        }
        match outcome {
            Ok(ack) => {
                self.local_backend.shutdown_message = Some(format!(
                    "Embedded backend stopped cleanly (checkpointed={}, relinquished_leases={}).",
                    ack.checkpointed, ack.relinquished_leases
                ));
                self.local_backend.last_shutdown_ack = Some(ack);
            }
            Err(error) => {
                self.local_backend.shutdown_message =
                    Some(format!("Embedded backend shutdown notice: {error}"));
            }
        }
    }

    pub(crate) fn readiness_pending(&self) -> bool {
        self.readiness_request.is_some()
    }

    pub fn refresh_readiness(&mut self) {
        if !self.connected() {
            self.readiness =
                Load::Failed("Connect to a compatible gateway to refresh checks".into());
            return;
        }
        self.readiness_revision = self.readiness_revision.wrapping_add(1);
        self.readiness_refresh_at = self.clock + 2.0;
        self.readiness_request = None;
        if !matches!(self.readiness, Load::Ready(_)) {
            self.readiness = Load::Loading;
        }
        self.record(Intent::Readiness);
        if let (Some(services), Some(client)) = (&self.services, &self.client) {
            let client = match &self.registration.context {
                Some(context) => match client.with_registered_context(context) {
                    Ok(client) => client,
                    Err(error) => {
                        self.readiness = Load::Failed(error.to_string());
                        return;
                    }
                },
                None => client.clone(),
            };
            self.readiness_request =
                services.readiness(client, self.connection.epoch, self.readiness_revision);
        }
    }

    pub(crate) fn complete_readiness(&mut self, result: Result<WorkstationReadiness, String>) {
        self.readiness_request = None;
        let kept = matches!(self.readiness, Load::Ready(_));
        match result {
            Ok(readiness) if readiness.deployment_id == self.connection.deployment_label() => {
                self.readiness_received_at = Some(self.clock);
                self.readiness_error = None;
                self.readiness = Load::Ready(readiness);
            }
            Ok(_) => {
                // Another deployment's checks are never kept.
                self.readiness_received_at = None;
                self.readiness_error = None;
                self.readiness =
                    Load::Failed("Gateway returned readiness for a different deployment".into());
            }
            Err(error) if kept => {
                // A transient failure keeps the last same-scope checks, now marked stale: with no
                // fresh observation every check reads as stale and repairs stay restricted.
                self.readiness_received_at = None;
                self.readiness_error = Some(error);
            }
            Err(error) => {
                self.readiness_error = None;
                self.readiness = Load::Failed(error);
            }
        }
        self.finish_dataset_preview();
    }

    pub fn connected(&self) -> bool {
        self.connection.status == ConnectionStatus::Connected
    }

    pub(crate) fn has_active_confirmation(&self) -> bool {
        self.confirm_maintenance_dialog
            || self.confirm_cycle_delete
            || self.maintenance.confirmation.is_some()
            || self.cycles.confirmation.is_some()
            || self.configuration.confirmation.is_some()
            || self.job_actions.confirmation.is_some()
            || self.classifier_activation.confirmation.is_some()
            || self.transfer.confirmation.is_some()
            || self.rebind.confirmation.is_some()
            || self.connection_profiles.confirmation.is_some()
            || self.workspaces.confirmation.is_some()
            || self
                .submissions
                .lane(SubmissionKind::Atlas)
                .confirmation
                .is_some()
            || self
                .submissions
                .lane(SubmissionKind::Tool)
                .confirmation
                .is_some()
    }

    pub(crate) fn maintenance_checks_due(&self, now: f64) -> bool {
        if !(matches!(self.page, Page::Seed | Page::Workstation) && !self.setup_gateway
            || self.startup.data_setup_active())
            || !self.connected()
            || !self.scoped_logs.window_focused
            || self.readiness_request.is_some()
            || now < self.readiness_refresh_at
        {
            return false;
        }
        if self.startup_data_needs_observation() {
            return true;
        }
        match &self.readiness {
            Load::Ready(readiness) => readiness.checks.iter().any(|check| {
                check.state == ReadinessState::Running || !self.readiness_check_fresh_at(check, now)
            }),
            Load::Idle | Load::Failed(_) => matches!(self.page, Page::Seed | Page::Workstation),
            Load::Pending | Load::Loading => false,
        }
    }

    pub(crate) fn job_list_has_active(&self) -> bool {
        matches!(
            &self.backend.jobs,
            Load::Ready(jobs)
                if jobs
                    .rows
                    .iter()
                    .any(|job| crate::backend::job_is_active(&job.state))
        )
    }

    pub(crate) fn selected_job_is_active(&self) -> bool {
        if let Load::Ready(detail) = &self.backend.detail {
            if self.backend.selected_job.as_deref() == Some(detail.job.id.as_str()) {
                return crate::backend::job_is_active(&detail.job.state);
            }
        }
        if let (Some(id), Load::Ready(jobs)) = (&self.backend.selected_job, &self.backend.jobs) {
            return jobs
                .rows
                .iter()
                .any(|job| job.id == *id && crate::backend::job_is_active(&job.state));
        }
        false
    }

    pub(crate) fn jobs_poll_due(&self, now: f64) -> bool {
        self.page == Page::Jobs
            && !self.fixture
            && self.connected()
            && self.scoped_logs.window_focused
            && (self.job_list_has_active()
                || self.selected_job_is_active()
                || (self.backend.selected_job.is_some()
                    && self.scoped_logs.status == crate::logs::LogStatus::Unavailable))
            && now >= self.jobs_refresh_at
    }

    /// Re-reads the registered workstation's current Jobs page without touching the selection,
    /// the page, or an in-flight read.
    pub(crate) fn discover_registered_jobs(&mut self, now: f64) {
        self.jobs_discovery_at = now + JOBS_DISCOVERY_SECS;
        if self.registration.context.is_some() && !self.scoped_jobs.busy() {
            self.request_jobs(crate::jobs_query::JobsEvent::Poll);
        }
    }

    pub(crate) fn refresh_active_job_status(&mut self, now: f64) {
        self.jobs_refresh_at = now + 2.0;
        if let Some(id) = self.backend.selected_job.clone() {
            if self.selected_job_is_active()
                && !self.backend.in_flight(crate::backend::ReadKind::JobDetail)
            {
                self.backend_read(crate::backend::ReadRequest::JobDetail { id });
            }
        }
        if !self.scoped_jobs.busy()
            && (self.job_list_has_active() || self.backend.selected_job.is_none())
        {
            self.request_jobs(crate::jobs_query::JobsEvent::Poll);
        }
        self.sync_operational_logs_with_selected_job(false);
    }

    pub(crate) fn atlas_poll_active(&self) -> bool {
        let lane = self
            .submissions
            .lane(crate::submission::SubmissionKind::Atlas);
        if lane.busy() || lane.receipt.is_some() {
            return true;
        }
        matches!(
            &self.cycles.history,
            Load::Ready(history)
                if history
                    .rows
                    .iter()
                    .any(|row| crate::backend::job_is_active(&row.state))
        )
    }

    pub fn search_available(&self) -> bool {
        self.connected()
            && self
                .connection
                .capabilities
                .as_ref()
                .is_some_and(|capabilities| capabilities.require_actor_search().is_ok())
    }

    pub fn query_edited(&mut self, tx: &mut Tx) {
        self.brain.edited(tx.clock());
        self.dossier_request = None;
        if self.brain.pending_at.is_some() {
            tx.wake_after(Duration::from_millis(200));
        }
    }

    pub fn search_now(&mut self) {
        if self.brain.query.text.trim().is_empty() {
            self.brain.results = Load::Idle;
            self.brain.pending_at = None;
            return;
        }
        if !self.search_available() {
            self.brain.results = Load::Failed(
                "This gateway does not provide the required actor discovery contract".into(),
            );
            self.brain.pending_at = None;
            return;
        }
        if self.brain.in_flight {
            return;
        }
        let query = match ActorSearchQuery::new(self.brain.query.text.trim().to_owned(), 20) {
            Ok(query) => query,
            Err(error) => {
                self.brain.results = Load::Failed(error.to_string());
                self.brain.pending_at = None;
                return;
            }
        };
        self.brain.pending_at = None;
        if !self.fixture && (self.services.is_none() || self.client.is_none()) {
            self.brain.results = Load::Failed("Gateway search services are unavailable".into());
            return;
        }
        self.brain.results = Load::Loading;
        self.brain.in_flight = true;
        self.record(Intent::Search {
            query: query.query.clone(),
        });
        if let (Some(services), Some(client)) = (&self.services, &self.client) {
            self.search_request = services.search(
                client.clone(),
                query,
                self.connection.epoch,
                self.brain.query_revision,
            );
        }
    }

    /// Open one result at a time; collapsing fences any pending dossier response.
    pub fn toggle_actor(&mut self, index: usize) {
        let Load::Ready(response) = &self.brain.results else {
            return;
        };
        let Some(hit) = response.rows.get(index) else {
            return;
        };
        if self.brain.showing_detail && self.brain.selected_actor.as_deref() == Some(&hit.actor_id)
        {
            self.brain.showing_detail = false;
            self.brain.selected_actor = None;
            self.brain.dossier = Load::Idle;
            self.brain.detail_revision = self.brain.detail_revision.wrapping_add(1);
            self.dossier_request = None;
        } else {
            self.select_actor(index);
        }
    }

    pub fn retry_dossier(&mut self) {
        let Load::Ready(response) = &self.brain.results else {
            return;
        };
        if !self.brain.showing_detail {
            return;
        }
        if let Some(index) = response
            .rows
            .iter()
            .position(|hit| Some(hit.actor_id.as_str()) == self.brain.selected_actor.as_deref())
        {
            self.select_actor(index);
        }
    }

    pub fn select_actor(&mut self, index: usize) {
        let Load::Ready(response) = &self.brain.results else {
            return;
        };
        let Some(hit) = response.rows.get(index) else {
            return;
        };
        let id = hit.actor_id.clone();
        self.brain.highlighted = Some(index);
        self.brain.selected_actor = Some(id.clone());
        self.brain.detail_revision = self.brain.detail_revision.wrapping_add(1);
        self.brain.showing_detail = true;
        self.brain_facet = DossierFacet::Overview;
        if !self.fixture && (self.services.is_none() || self.client.is_none()) {
            self.brain.dossier = Load::Failed("Gateway dossier services are unavailable".into());
            return;
        }
        self.brain.dossier = Load::Loading;
        self.dossier_request = None;
        self.record(Intent::Dossier {
            actor_id: id.clone(),
        });
        if let (Some(services), Some(client)) = (&self.services, &self.client) {
            self.dossier_request = services.dossier(
                client.clone(),
                id,
                self.connection.epoch,
                self.brain.detail_revision,
            );
        }
    }

    pub fn search_key(&mut self, event: &KeyEvent, repeat: bool) -> bool {
        match event.key {
            Key::Down => self.brain.move_highlight(true),
            Key::Up => self.brain.move_highlight(false),
            Key::Enter if repeat => {}
            Key::Enter => {
                if let Some(index) = self.brain.highlighted {
                    self.toggle_actor(index);
                } else {
                    self.search_now();
                }
            }
            _ => return false,
        }
        true
    }

    pub(crate) fn record(&mut self, intent: Intent) {
        if self.fixture {
            self.intents.push(intent.clone());
            if let Some(effect) = self.fixture_effect {
                effect(self, &intent);
            }
        }
    }
}

/// Seconds between discovery reads when no active job is known on the Jobs page.
const JOBS_DISCOVERY_SECS: f64 = 5.0;
/// Low-cadence registered aggregate refresh while focused on another page.
const JOBS_BACKGROUND_DISCOVERY_SECS: f64 = 15.0;

/// Jobs scheduling. On the Jobs page, two independent loops share the tick:
/// - follow known active work, or recover unavailable logs of the selected job;
/// - discover newly admitted work for the registered workstation.
///
/// Off the Jobs page, refresh the shared registered job aggregate at a bounded low cadence
/// while focused without changing the current list query or scroll position.
pub(crate) fn poll_jobs(state: &mut State, now: f64, tx: &mut Tx) -> bool {
    if state.fixture
        || !state.connected()
        || !state.scoped_logs.window_focused
        || state.setup_gateway
    {
        return false;
    }
    if state.page != Page::Jobs {
        if state.registration.context.is_some()
            && state.permission(fleetworks_contracts::context::Permission::ReadWorkstationActivity)
        {
            let mut changed = false;
            if now >= state.jobs_discovery_at {
                state.jobs_discovery_at = now + JOBS_BACKGROUND_DISCOVERY_SECS;
                if !state.scoped_jobs.busy() {
                    state.request_jobs(crate::jobs_query::JobsEvent::Poll);
                    changed = true;
                }
            }
            tx.wake_after(Duration::from_secs(5));
            return changed;
        }
        return false;
    }
    let mut changed = false;
    if state.job_list_has_active()
        || state.selected_job_is_active()
        || (state.backend.selected_job.is_some()
            && state.scoped_logs.status == crate::logs::LogStatus::Unavailable)
    {
        if state.jobs_poll_due(now) {
            state.refresh_active_job_status(now);
            changed = true;
        }
        tx.wake_after(Duration::from_secs(1));
    }
    // A list with active rows is already re-read by the active path above.
    if state.registration.context.is_some() && !state.job_list_has_active() {
        if now >= state.jobs_discovery_at {
            state.discover_registered_jobs(now);
            changed = true;
        }
        tx.wake_after(Duration::from_secs(1));
    }
    changed
}

fn poll(state: &mut State, now: f64, tx: &mut Tx) -> bool {
    state.clock = now;
    let mut changed = false;
    if state.startup.active() && !state.setup_gateway {
        state.startup.begin(now);
        if let Some(wait) = state.startup.next_wake(now) {
            tx.wake_after(Duration::from_secs_f64(wait));
        }
        // Redraw the orbit's final pose even when this poll schedules no subsequent frame.
        return true;
    }
    changed |= state.startup.poll_flight(now, tx);
    changed |= state.drive_startup_dataset();
    changed |= state.observe_inbox();
    changed |= state.poll_inbox_qa(now, tx);
    changed |= state.sync_assistant();
    changed |= state.poll_scoped_logs();
    changed |= state.sync_jobs_binding();
    if (matches!(state.page, Page::Seed | Page::Workstation) && !state.setup_gateway
        || state.startup.data_setup_active())
        && !state.fixture
        && state.connected()
        && state.scoped_logs.window_focused
    {
        if state.maintenance_checks_due(now) {
            state.refresh_readiness();
            changed = true;
        }
        if !state.maintenance.active_receipt_jobs().is_empty()
            && now >= state.maintenance_jobs_refresh_at
        {
            state.maintenance_jobs_refresh_at = now + 2.0;
            if state.poll_maintenance_receipt_jobs() > 0 {
                changed = true;
            }
        }
        tx.wake_after(Duration::from_secs(2));
    }
    changed |= poll_jobs(state, now, tx);
    if state.page == Page::Workstation
        && !state.fixture
        && state.connected()
        && state.scoped_logs.window_focused
        && state.permission(fleetworks_contracts::context::Permission::ReadBackend)
    {
        if now >= state.host_metrics_refresh_at && state.host_metrics_request.is_none() {
            state.host_metrics_refresh_at = now + 5.0;
            state.refresh_host_metrics();
            changed = true;
        }
        tx.wake_after(Duration::from_secs(2));
    }
    if state.page == Page::Atlas
        && !state.fixture
        && state.connected()
        && state.scoped_logs.window_focused
        && state.registration.context.is_some()
    {
        if !state.setup_gateway && state.atlas_preflight_due() {
            state.refresh_readiness();
            changed = true;
        }
        // Active work is read quickly. Otherwise a slower, bounded read still discovers cycles
        // admitted after this page loaded, including from an empty or finished-only page.
        let interval = if state.atlas_poll_active() { 3.0 } else { 10.0 };
        if now >= state.atlas_refresh_at && !state.cycles.busy() {
            state.atlas_refresh_at = now + interval;
            if matches!(state.cycles.history, Load::Ready(_)) {
                state.poll_cycles();
            } else {
                state.refresh_cycles();
            }
            changed = true;
        }
        tx.wake_after(Duration::from_secs(1));
    }
    if let Some(key) = state.reveal.take() {
        tx.scroll_into_view(key);
        changed = true;
    }
    state.restore_workstation_index_focus(tx);
    if !state.brain.in_flight
        && state
            .brain
            .pending_at
            .is_some_and(|deadline| deadline <= now)
    {
        state.search_now();
        changed = true;
    }
    changed
}

fn on_key(state: &mut State, event: &KeyEvent, tx: &mut Tx) -> bool {
    if state.startup.active() {
        if event.key == Key::Escape && state.setup_gateway {
            if state.startup.data_setup_active() {
                if state.maintenance.confirmation.is_some() {
                    state.dismiss_maintenance_confirmation(tx);
                } else if state.maintenance.csv_path_prompt.is_some() {
                    state.cancel_csv_path_prompt(tx);
                } else {
                    return false;
                }
            } else {
                state.return_to_startup_choice(tx);
            }
            return true;
        }
        return false;
    }
    if state.inbox.open {
        if event.key == Key::Escape {
            state.close_inbox(tx);
            return true;
        }
        return false;
    }
    if event.is_command('k') && !event.mods.shift {
        if state.commands.open {
            state.commands.close();
            tx.focus("command-open", false);
        } else if !state.has_active_confirmation() {
            state.open_command_panel();
        }
        return true;
    }
    if event.key == Key::Escape {
        if state.commands.open {
            state.commands.close();
            tx.focus("command-open", false);
            return true;
        }
        if state.confirm_maintenance_dialog || state.maintenance.confirmation.is_some() {
            state.dismiss_maintenance_confirmation(tx);
            return true;
        }
        if state.settings_open {
            state.settings_open = false;
            tx.focus("open-workstation-settings", false);
            return true;
        }
        if state.workstation_inspector.is_some() {
            state.workstation_inspector = None;
            return true;
        }
        if state.setup_gateway {
            state.setup_gateway = false;
            tx.focus("connection-editor", false);
            return true;
        }
    }
    if shell::is_sidebar_toggle(event) {
        state.sidebar.toggle();
        return true;
    }
    if state.connected() {
        for (index, page) in Page::ALL.into_iter().enumerate() {
            if event.is_command(char::from(b'1' + index as u8)) {
                state.navigate(page);
                return true;
            }
        }
    }
    if event.is_command('f') && state.page == Page::Brain {
        tx.focus("actor-search/value/input", true);
        return true;
    }
    false
}

/// The actual production app specification; fixture scenes are added only by the example.
pub fn spec() -> AppSpec<State> {
    AppSpec {
        min_size: Some((640.0, 480.0)),
        poll: Some(poll),
        on_key: Some(on_key),
        ..AppSpec::new("Fleetworks", (1440.0, 860.0), State::default, view)
    }
}

/// Keep the surface readable and its input available during the brief entrance.
pub(crate) fn surface_transition(reduced_motion: bool, mut transition: Transition) -> Transition {
    if reduced_motion {
        return Transition::none();
    }
    transition.insertion.opacity = 0.8;
    transition.springs(SpringPair::new(
        Spring::new(0.16, 0.0),
        Spring::new(0.12, 0.0),
    ))
}

/// Shared view used unchanged by the production executable and native fixture example.
pub fn view(state: &State, cx: &mut Cx<State>) -> El {
    let preferences = state
        .accessibility
        .for_view(state.fixture, cx.is_reduced_motion());
    if preferences.reduced_motion {
        cx.reduced_motion(true);
    }
    let mut kit = theme::kit();
    let mut layers = Vec::new();
    crate::material::configure_kit(&mut kit, preferences);
    if state.startup.active() {
        let startup = crate::startup::view(state, cx, preferences, &kit);
        return mark_fixture(state, &kit, startup).on_window_focus(State::log_window_focus);
    }
    let shell_width = cx.layout_size("app-shell").map(|size| size[0]);
    let compact_rail = shell_width.is_some_and(|width| width < 820.0);
    let mut selected_rail_kit = kit.clone();
    selected_rail_kit.c.pressed = theme::hex(theme::MOLTEN_SELECTION);
    selected_rail_kit.c.text_base = theme::hex(theme::MOLTEN_ACCENT);
    selected_rail_kit.c.icon_base = theme::hex(theme::MOLTEN_ACCENT);
    selected_rail_kit.radius.md = 999.0;

    let connection_tone = match &state.connection.status {
        ConnectionStatus::Connected => StatusTone::Ok,
        ConnectionStatus::Connecting => StatusTone::Warn,
        ConnectionStatus::Failed(_) => StatusTone::Err,
        ConnectionStatus::Disconnected => StatusTone::Neutral,
    };
    let sidebar_el = (!compact_rail).then(|| {
        let row_kit = |selected| if selected { &selected_rail_kit } else { &kit };
        let workstation_selected = !state.setup_gateway && state.page == Page::Workstation;
        let mut sidebar = Sidebar::new()
            .label("Fleetworks pages")
            .header_gap(6.0)
            .header_padding(4.0, 12.0)
            .header(
                rune::row()
                    .w_full()
                    .px(8.0)
                    .py(10.0)
                    .gap(8.0)
                    .center_y()
                    // The approved symbol alone: no wordmark. Its accessible name stays "Fleetworks".
                    .child(crate::startup::rail_mark(state, 34.0, preferences))
                    .child(rune::spacer())
                    .child(ui_components::status_dot(connection_tone, preferences)),
            )
            .child(
                Button::new(match &state.connection.status {
                    ConnectionStatus::Connected => "Gateway connected",
                    ConnectionStatus::Connecting => "Connecting…",
                    ConnectionStatus::Failed(_) => "Connection fault",
                    ConnectionStatus::Disconnected => "Gateway disconnected",
                })
                .variant(ButtonVariant::Ghost)
                .icon(&icons::SERVER)
                .on_press(|state: &mut State| {
                    if state.connected() {
                        state.navigate(Page::Workstation);
                    } else {
                        state.setup_gateway = true;
                    }
                })
                .build(&kit)
                .key("gateway-status-header"),
            )
            .child(
                SidebarHeader::new("Workstation")
                    .build(&kit)
                    .key("sidebar-workstation-heading"),
            )
            .child(
                SidebarRow::new(Page::Workstation.title())
                    .icon(Page::Workstation.icon())
                    .selected(workstation_selected)
                    .on_select(|state: &mut State, _| state.navigate(Page::Workstation))
                    .build(row_kit(workstation_selected))
                    .key("nav-workstation"),
            );

        if state.connected() {
            sidebar = sidebar.child(
                SidebarHeader::new("Operations")
                    .build(&kit)
                    .key("sidebar-operations-heading"),
            );
            for page in Page::ALL.into_iter().filter(|page| *page != Page::Seed) {
                let selected = !state.setup_gateway && state.page == page;
                let mut row_builder = SidebarRow::new(page.title())
                    .icon(page.icon())
                    .selected(selected)
                    .on_select(move |state: &mut State, _| state.navigate(page));
                if let Some(status) = sidebar_row_status(state, page) {
                    row_builder = row_builder.status(status);
                }
                if let Some(badge) = sidebar_row_badge(state, page) {
                    row_builder = row_builder.badge(badge);
                }
                sidebar = sidebar.child(
                    row_builder
                        .build(row_kit(selected))
                        .key(format!("nav-{}", page.key())),
                );
            }
        } else {
            sidebar = sidebar.child(
                SidebarRow::new("Setup Gateway")
                    .icon(&icons::KEY)
                    .selected(state.setup_gateway)
                    .on_select(|state: &mut State, _| state.setup_gateway = true)
                    .build(row_kit(state.setup_gateway))
                    .key("nav-setup"),
            );
        }
        sidebar
            .footer(
                Button::new("Settings")
                    .variant(ButtonVariant::Ghost)
                    .icon(&icons::WORKSTATION)
                    .on_press(|state: &mut State| {
                        state.settings_open = true;
                    })
                    .build(&kit)
                    .key("open-workstation-settings"),
            )
            .build(&kit)
    });

    let scope = if state.page.is_backend() {
        "Backend-wide".to_owned()
    } else if state.registration.context.is_some() {
        "Workstation activity".to_owned()
    } else {
        "Workstation activity: registration required".to_owned()
    };
    // The chat aux panel stays off until the typed chat contract lands. The original context
    // rail keeps its place meanwhile.
    let assistant_room = if state.assistant.enabled && state.connected() && !state.setup_gateway {
        cx.layout_with("main-area", |rect| rect[2])
    } else {
        None
    };
    let assistant_placement = assistant_room.map_or(crate::assistant::Placement::Hidden, |room| {
        state.assistant.preference.placement(room)
    });
    let toolbar = workstation_toolbar(
        state,
        cx,
        &kit,
        preferences,
        sidebar_el.is_some(),
        assistant_placement,
        connection_tone,
    );

    let any_busy = state.connection.status == ConnectionStatus::Connecting
        || state.pairing.busy()
        || matches!(state.readiness, Load::Pending | Load::Loading)
        || matches!(state.host_metrics, Load::Pending | Load::Loading)
        || state.brain.in_flight
        || state.maintenance.busy()
        || state.configuration.request.is_some()
        || state.backend.busy();

    let mut content = rune::col()
        .key("page-content")
        .h_full()
        .grow(1.0)
        .min_w(0.0)
        .min_h(0.0);
    if !state.setup_gateway {
        let mut scope_bar = rune::row().w_full().px(20.0).py(10.0).gap(10.0).center_y();
        if state.connected() {
            scope_bar = scope_bar.child(
                rune::text(scope)
                    .key("resource-scope")
                    .font(kit.ty.small_medium)
                    .color(kit.c.text_accent),
            );
        } else {
            scope_bar = scope_bar.child(
                rune::text("Local Client")
                    .key("local-client-scope")
                    .font(kit.ty.small_medium)
                    .color(kit.c.text_muted),
            );
        }
        scope_bar = scope_bar.child(rune::spacer());
        // Connection health never changes with request activity. A reserved slot holds a small
        // spinner while any read is in flight, so the pill and the bar keep their size.
        let activity = if any_busy {
            rune_kit::Spinner::new()
                .size(13.0)
                .color(theme::hex(theme::TEXT_MUTED))
                .build(&kit)
        } else {
            rune::el()
        };
        scope_bar = scope_bar
            .child(
                rune::row()
                    .key("activity-slot")
                    .w(13.0)
                    .h(13.0)
                    .a11y_hidden()
                    .child(activity),
            )
            .child(ui_components::status_pill(
                match &state.connection.status {
                    ConnectionStatus::Connected => "Online",
                    ConnectionStatus::Connecting => "Connecting",
                    ConnectionStatus::Failed(_) => "Fault",
                    ConnectionStatus::Disconnected => "Offline",
                },
                match &state.connection.status {
                    ConnectionStatus::Disconnected => Some(&icons::SERVER),
                    _ => None,
                },
                connection_tone,
                &kit,
            ));
        content = content.child(
            material::glass(scope_bar, GlassSurface::Chrome, preferences).backdrop_blur(0.0),
        );
    }

    let screen_key = if state.setup_gateway {
        "screen-setup".to_owned()
    } else if !state.connected() {
        format!("screen-{}", Page::Workstation.key())
    } else {
        format!("screen-{}", state.page.key())
    };
    let page_body = views::page(state, cx, &kit, &mut layers);
    let floating_layer_open =
        !layers.is_empty() || state.settings_open || state.inbox.open || state.commands.open;
    let chrome = |el| {
        let glass = material::glass(el, GlassSurface::Chrome, preferences);
        if floating_layer_open {
            // Leave Rune's limited backdrop slots available for the open overlay.
            glass.backdrop_blur(0.0)
        } else {
            glass
        }
    };

    // Only the content changes identity on navigation. Keep the shell, toolbar and canvas in place.
    let page_transition = if state.setup_gateway {
        Transition::offset(0.0, 5.0)
    } else {
        Transition::identity()
    };
    let mut page_transition = surface_transition(preferences.reduced_motion, page_transition);
    // Discard the old page immediately, including when its connection scope just changed.
    page_transition.springs.exit = Spring::NONE;
    let screen = rune::col()
        .key(screen_key)
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .transition(page_transition)
        .child(page_body);
    let screen = if preferences.reduced_motion {
        screen.instant_changes()
    } else {
        screen
    };
    let content = content.child(screen);
    let mut main = rune::row().key("main-area").w_full().h_full().min_h(0.0);
    if compact_rail {
        main = main.child(chrome(compact_icon_rail(
            state,
            &kit,
            &selected_rail_kit,
            connection_tone,
            preferences,
        )));
    }
    main = main.child(content);
    if !state.assistant.enabled
        && state.connected()
        && !state.setup_gateway
        && cx
            .layout_with("main-area", |rect| rect[2] >= 1120.0)
            .unwrap_or(false)
    {
        main = main.child(context_rail(state, &kit, connection_tone));
    }
    match assistant_placement {
        crate::assistant::Placement::Docked => {
            main = main.child(crate::assistant::dock(state, &kit, preferences));
        }
        crate::assistant::Placement::Overlay => {
            // Too narrow to share the window: float the dock over the right edge.
            layers.push(
                rune::row()
                    .key("assistant-overlay")
                    .transition(if preferences.reduced_motion {
                        Transition::none()
                    } else {
                        Transition::identity()
                    })
                    .absolute(0.0, 0.0)
                    .w_full()
                    .h_full()
                    .child(
                        rune::el()
                            .key("assistant-scrim")
                            .grow(1.0)
                            .h_full()
                            .on_press(|state: &mut State| {
                                state.assistant.preference = crate::assistant::Preference::Closed;
                            }),
                    )
                    .child(
                        crate::assistant::dock(state, &kit, preferences)
                            .focus_return("assistant-toggle")
                            .transition(surface_transition(
                                preferences.reduced_motion,
                                Transition::offset(6.0, 0.0),
                            ))
                            .instant_changes(),
                    ),
            );
        }
        crate::assistant::Placement::Hidden => {}
    }

    if state.settings_open {
        let width = cx
            .layout_size("main-area")
            .map(|size| (size[0] - 24.0).clamp(420.0, 940.0))
            .unwrap_or(760.0);
        layers.extend(crate::ui_components::glass_dialog(
            "workstation-settings",
            "Settings",
            width,
            crate::workstation_settings::page(state, cx, &kit),
            true,
            |state: &mut State| state.settings_open = false,
            preferences,
            &kit,
        ));
    }

    if !state.has_active_confirmation() {
        crate::command_panel::view(state, &kit, preferences, compact_rail, &mut layers);
    }

    if let Some(drawer) = crate::inbox::drawer(state, cx, &kit, preferences) {
        layers.push(drawer);
    }

    let mut split = SplitView::new(main)
        .toolbar(toolbar)
        .titlebar_inset(cx.titlebar_inset())
        .reduced_motion(cx.is_reduced_motion());
    if let Some(sidebar_el) = sidebar_el {
        split = split.sidebar(
            &state.sidebar,
            |state: &mut State| &mut state.sidebar,
            chrome(sidebar_el),
        );
    }
    // The split shell owns the controls, window drag region, and sidebar grip. Paint its
    // full-width top band as a sibling underlay so those hit regions stay in place.
    let toolbar_glass = chrome(
        rune::el()
            .key("toolbar-glass")
            .absolute(0.0, 0.0)
            .w_full()
            .h(TOOLBAR_HEIGHT)
            .paint_order(-1)
            .a11y_hidden(),
    );
    let mut shell_kit = kit.clone();
    shell_kit.c.bg_base = theme::hex(theme::BACKGROUND);
    let base = split
        .build(&shell_kit)
        .key("app-shell")
        .child(toolbar_glass);
    let base = mark_fixture(state, &kit, base);
    if let Some(mark) = crate::startup::rail_handoff(state, cx, preferences) {
        layers.insert(0, mark);
    }
    rune_kit::root(base, layers).on_window_focus(State::log_window_focus)
}

fn mark_fixture(state: &State, kit: &rune_kit::Kit, view: El) -> El {
    if !state.fixture {
        return view;
    }
    rune::col()
        .w_full()
        .h_full()
        .child(
            rune::text("Fixture — synthetic data, boundary effects disabled")
                .key("fixture-marker")
                .font(kit.ty.small)
                .color(kit.c.info.fg)
                .padding(6.0),
        )
        .child(view.grow(1.0).min_h(0.0))
}

fn workstation_toolbar(
    state: &State,
    cx: &Cx<State>,
    kit: &rune_kit::Kit,
    preferences: ViewPreferences,
    has_sidebar: bool,
    assistant_placement: crate::assistant::Placement,
    tone: StatusTone,
) -> Toolbar {
    use crate::command_panel::TOOLBAR_COMMAND_COMPACT_WIDTH;
    use crate::command_panel::TOOLBAR_COMMAND_LABELED_WIDTH;

    const TRAILING_GAP: f32 = 6.0;
    const TOOLBAR_EDGE_INSET: f32 = 12.0;
    const TOOLBAR_FLOW_GAP: f32 = 8.0;
    const TRAILING_CLEARANCE: f32 = TOOLBAR_EDGE_INSET + 2.0 * TOOLBAR_FLOW_GAP;
    const GATEWAY_ICON: f32 = 32.0;
    const GATEWAY_LABELED: f32 = 152.0;
    const TELEMETRY_COMPACT: f32 = 76.0;
    const TELEMETRY_LABELED: f32 = 244.0;
    const ASSISTANT_WIDTH: f32 = 24.0;

    // Match the outer padding and the two 8pt flow gaps on either side of the
    // Rune Kit toolbar's flexible spacer. When the sidebar is open the toolbar
    // starts at its inner edge, and SplitView supplies the standard leading pad.
    let open_sidebar = has_sidebar && state.sidebar.is_open();
    let shell_width = cx.layout_size("app-shell").map_or(640.0, |size| size[0]);
    let width = shell_width
        - if open_sidebar {
            state.sidebar.shown_width()
        } else {
            0.0
        };
    let leading_inset = if open_sidebar {
        TOOLBAR_EDGE_INSET
    } else {
        let lights = cx.titlebar_inset();
        let edge = if lights > 0.0 {
            lights + 8.0
        } else {
            TOOLBAR_EDGE_INSET
        };
        edge + if has_sidebar { 32.0 } else { 0.0 }
    };
    let show_assistant = state.assistant.enabled && state.connected() && !state.setup_gateway;
    let bell_width = if state.inbox.unread() == 0 {
        38.0
    } else {
        66.0
    };
    let trailing_width = |gateway, telemetry| {
        let assistant = if show_assistant {
            ASSISTANT_WIDTH + TRAILING_GAP
        } else {
            0.0
        };
        let metrics = if telemetry > 0.0 {
            telemetry + TRAILING_GAP
        } else {
            0.0
        };
        gateway + bell_width + TRAILING_GAP + assistant + metrics
    };
    let fits = |command, gateway, telemetry| {
        let half_room = (width - command) / 2.0;
        half_room >= leading_inset + TOOLBAR_FLOW_GAP
            && half_room >= trailing_width(gateway, telemetry) + TRAILING_CLEARANCE
    };
    let telemetry_width = if !state.connected() {
        0.0
    } else if fits(
        TOOLBAR_COMMAND_COMPACT_WIDTH,
        GATEWAY_ICON,
        TELEMETRY_LABELED,
    ) {
        TELEMETRY_LABELED
    } else if width
        >= leading_inset
            + TOOLBAR_COMMAND_COMPACT_WIDTH
            + trailing_width(GATEWAY_ICON, TELEMETRY_COMPACT)
            + TRAILING_CLEARANCE
    {
        TELEMETRY_COMPACT
    } else {
        // Before the compact rail is measured, the full rail can consume this space.
        0.0
    };
    let wide_gateway = fits(
        TOOLBAR_COMMAND_LABELED_WIDTH,
        GATEWAY_LABELED,
        telemetry_width,
    );
    let gateway_width = if wide_gateway {
        GATEWAY_LABELED
    } else {
        GATEWAY_ICON
    };
    let compact_command = !fits(
        TOOLBAR_COMMAND_LABELED_WIDTH,
        gateway_width,
        telemetry_width,
    ) || (state.connected() && telemetry_width == TELEMETRY_COMPACT);
    let command_width = if compact_command {
        TOOLBAR_COMMAND_COMPACT_WIDTH
    } else {
        TOOLBAR_COMMAND_LABELED_WIDTH
    };
    let title_width = ((width - command_width) / 2.0 - leading_inset).max(0.0);
    let title_page = if state.connected() {
        state.page
    } else {
        Page::Workstation
    };
    let title = match (state.setup_gateway, title_width < 140.0) {
        (true, true) => "Setup",
        (true, false) => "Setup Gateway",
        (false, true) => title_page.compact_title(),
        (false, false) => title_page.title(),
    };
    let leading = rune::row()
        .shrink()
        .center_y()
        .child(
            rune::row().w(title_width).pr(8.0).shrink().child(
                rune::text(title)
                    .font(kit.ty.ui_medium)
                    .color(kit.c.text_base)
                    .truncate(),
            ),
        )
        .child(crate::command_panel::toolbar_button(
            kit,
            compact_command,
            preferences,
        ));
    let mut trailing = rune::row().gap(TRAILING_GAP).center_y();
    if show_assistant {
        let toggle = crate::assistant::toggle(
            assistant_placement != crate::assistant::Placement::Hidden,
            kit,
        )
        .w(ASSISTANT_WIDTH);
        trailing = trailing.child(material::glass_control(toggle, preferences));
    }
    if telemetry_width > 0.0 {
        trailing = trailing.child(material::glass(
            toolbar_telemetry(state, kit, telemetry_width == TELEMETRY_COMPACT).w(telemetry_width),
            GlassSurface::Control,
            preferences,
        ));
    }
    let gateway = gateway_toolbar_button(state, kit, !wide_gateway, tone).w(gateway_width);
    Toolbar::new().leading(leading).trailing(
        trailing
            .child(material::glass_control(gateway, preferences))
            .child(crate::inbox::toolbar_item(state, kit, preferences).w(bell_width)),
    )
}

fn gateway_toolbar_button(
    state: &State,
    kit: &rune_kit::Kit,
    compact: bool,
    tone: StatusTone,
) -> El {
    let status = match state.connection.status {
        ConnectionStatus::Connected => "Gateway connected",
        ConnectionStatus::Connecting => "Gateway connecting",
        ConnectionStatus::Failed(_) => "Gateway fault",
        ConnectionStatus::Disconnected => "Gateway disconnected",
    };
    let label = if state.connection.status == ConnectionStatus::Disconnected {
        "Setup Gateway"
    } else {
        status
    };
    Button::new(format!("{status}. Open gateway setup"))
        .variant(ButtonVariant::Ghost)
        .help("Setup Gateway")
        .label_with(move |font, _| {
            let mut content = rune::row().gap(5.0).center_y().child(
                rune::icon(&icons::SERVER, 14.0)
                    .color(tone.fg())
                    .a11y_hidden(),
            );
            if !compact {
                content = content.child(rune::text(label).font(font).color(tone.fg()));
            }
            content
        })
        .on_press(|state: &mut State| state.setup_gateway = true)
        .build(kit)
        .key("connection-editor")
        .h(28.0)
        .px(8.0)
        .radius(999.0)
}

fn toolbar_telemetry(state: &State, kit: &rune_kit::Kit, compact: bool) -> El {
    let metrics = match &state.host_metrics {
        Load::Ready(metrics) => Some(metrics),
        _ => None,
    };
    let samples = [
        (
            "CPU",
            &icons::CPU,
            metrics.and_then(|value| value.cpu_percent),
        ),
        (
            "MEM",
            &icons::MEMORY,
            metrics.and_then(|value| value.memory_percent),
        ),
        (
            "DSK",
            &icons::DISK,
            metrics.and_then(|value| value.disk_percent),
        ),
    ]
    .map(|(name, icon, sample)| {
        (
            name,
            icon,
            sample.filter(|value| (0.0..=100.0).contains(value)),
        )
    });
    let stale = state.host_metrics_error.is_some();
    let percent =
        |sample: Option<f64>| sample.map_or_else(|| "—".into(), |value| format!("{value:.0}%"));
    let [cpu, memory, disk] = samples.map(|(_, _, sample)| percent(sample));
    let freshness = if stale { ", stale" } else { "" };
    let description =
        format!("Gateway telemetry, CPU {cpu}, memory {memory}, disk {disk}{freshness}");
    let ink = |sample| {
        theme::hex(match sample {
            Some(_) if stale => theme::TEXT_MUTED,
            Some(value) if value >= 95.0 => theme::STATUS_ERR,
            Some(value) if value >= 85.0 => theme::STATUS_WARN,
            Some(_) => theme::TEXT_PRIMARY,
            None => theme::TEXT_MUTED,
        })
    };
    let badge = |key, icon, label: String, sample| {
        rune::row()
            .key(key)
            .gap(4.0)
            .center_y()
            .a11y_hidden()
            .child(rune::icon(icon, 12.0).color(ink(sample)))
            .child(
                rune::text(label)
                    .font(kit.ty.small_medium)
                    .color(ink(sample)),
            )
    };
    let mut strip = rune::row()
        .key("toolbar-telemetry")
        .label(description.clone())
        .help(description)
        .role("group")
        .h(28.0)
        .px(8.0)
        .gap(9.0)
        .center_y()
        .radius(999.0);
    if compact {
        let busiest = samples
            .iter()
            .filter_map(|(_, icon, sample)| sample.map(|value| (*icon, value)))
            .max_by(|(_, left), (_, right)| left.total_cmp(right));
        let (icon, sample) = busiest.map_or((&icons::WORKSTATION, None), |(icon, value)| {
            (icon, Some(value))
        });
        strip.child(badge("telemetry-summary", icon, percent(sample), sample))
    } else {
        for (name, icon, sample) in samples {
            strip = strip.child(badge(
                name,
                icon,
                format!("{name} {}", percent(sample)),
                sample,
            ));
        }
        strip
    }
}

fn compact_icon_rail(
    state: &State,
    kit: &rune_kit::Kit,
    selected_kit: &rune_kit::Kit,
    connection_tone: StatusTone,
    preferences: ViewPreferences,
) -> El {
    let nav_icon = |icon, label: &'static str, selected| {
        let palette = if selected { selected_kit } else { kit };
        let fill = if selected {
            palette.c.pressed
        } else {
            rune::Color::TRANSPARENT
        };
        IconButton::new(icon)
            .variant(if selected {
                IconButtonVariant::Ghost
            } else {
                IconButtonVariant::GhostMuted
            })
            .control_size(ControlSize::Large)
            .label(label)
            .help(label)
            .build(palette)
            .fill(fill)
            .focus_selected(selected)
    };
    let mut rail = rune::col()
        .key("compact-icon-rail")
        .w(52.0)
        .h_full()
        .py(10.0)
        .px(6.0)
        .gap(6.0)
        .items_center()
        .child(crate::startup::rail_mark(state, 28.0, preferences))
        .child(
            rune::col()
                .py(4.0)
                .items_center()
                .child(ui_components::status_dot(connection_tone, preferences)),
        )
        .child(
            nav_icon(
                Page::Workstation.icon(),
                Page::Workstation.title(),
                !state.setup_gateway && state.page == Page::Workstation,
            )
            .on_press(|state: &mut State| state.navigate(Page::Workstation))
            .key("nav-workstation"),
        );
    if state.connected() {
        for page in Page::ALL.into_iter().filter(|page| *page != Page::Seed) {
            rail = rail.child(
                nav_icon(
                    page.icon(),
                    page.title(),
                    !state.setup_gateway && state.page == page,
                )
                .on_press(move |state: &mut State| state.navigate(page))
                .key(format!("nav-{}", page.key())),
            );
        }
    } else {
        rail = rail.child(
            nav_icon(&icons::KEY, "Setup Gateway", state.setup_gateway)
                .on_press(|state: &mut State| state.setup_gateway = true)
                .key("nav-setup"),
        );
    }
    rail.child(rune::spacer()).child(
        nav_icon(&icons::WORKSTATION, "Settings", state.settings_open)
            .on_press(|state: &mut State| state.settings_open = true)
            .key("open-workstation-settings"),
    )
}

fn context_rail(state: &State, kit: &rune_kit::Kit, connection_tone: StatusTone) -> El {
    let page = state.page;
    let scope = if page.is_backend() {
        "Shared backend resources"
    } else {
        "Registered workstation activity"
    };
    let working = sidebar_row_status(state, page).is_some();
    let mut activity = rune::col()
        .w_full()
        .gap(8.0)
        .child(
            rune::text("ACTIVITY")
                .font(kit.ty.text_xsmall_medium)
                .color(kit.c.text_faint),
        )
        .child(ui_components::status_pill(
            if working { "Working" } else { "Ready" },
            None,
            if working {
                StatusTone::Accent
            } else {
                connection_tone
            },
            kit,
        ));
    if let Some(count) = sidebar_row_badge(state, page) {
        let count_num = count.parse::<usize>().unwrap_or(0);
        let records = if page == Page::Jobs {
            if count_num == 1 {
                "1 job in progress".to_owned()
            } else {
                format!("{count} jobs in progress")
            }
        } else if count_num == 1 {
            "1 visible record".to_owned()
        } else {
            format!("{count} visible records")
        };
        activity = activity.child(
            rune::text(records)
                .font(kit.ty.small)
                .color(kit.c.text_muted),
        );
    }
    rune::col()
        .key("context-rail")
        .w(236.0)
        .h_full()
        .px(14.0)
        .py(18.0)
        .gap(18.0)
        .fill(theme::hex(theme::SURFACE))
        .border_sides([1.0, 0.0, 0.0, 0.0], theme::hex(theme::BORDER_SUBTLE))
        .child(
            rune::col()
                .w_full()
                .gap(9.0)
                .child(
                    rune::text("OVERVIEW")
                        .font(kit.ty.text_xsmall_medium)
                        .color(kit.c.text_faint),
                )
                .child(
                    rune::text(page.title())
                        .font(kit.ty.body_medium)
                        .color(kit.c.text_base)
                        .wrap(),
                )
                .child(
                    rune::text(page.summary_hint())
                        .font(kit.ty.small)
                        .color(kit.c.text_muted)
                        .wrap(),
                ),
        )
        .child(
            rune::col()
                .w_full()
                .pt(14.0)
                .gap(8.0)
                .border_sides([0.0, 0.0, 1.0, 0.0], theme::hex(theme::BORDER_SUBTLE))
                .child(
                    rune::text("SCOPE")
                        .font(kit.ty.text_xsmall_medium)
                        .color(kit.c.text_faint),
                )
                .child(
                    rune::text(scope)
                        .font(kit.ty.small)
                        .color(kit.c.text_muted)
                        .wrap(),
                ),
        )
        .child(
            activity
                .pt(14.0)
                .border_sides([0.0, 0.0, 1.0, 0.0], theme::hex(theme::BORDER_SUBTLE)),
        )
}

fn sidebar_row_status(state: &State, page: Page) -> Option<RowStatus> {
    let busy = match page {
        Page::Seed => state.maintenance.busy(),
        Page::Atlas => state.cycles.busy(),
        Page::Brain => state.brain.in_flight,
        Page::Tools => state.backend.busy(),
        Page::Models => state.configuration.request.is_some(),
        Page::Jobs => state.scoped_jobs.busy() || state.backend.busy(),
        Page::Profile | Page::Workstation | Page::Workspaces => false,
    };
    if busy {
        Some(RowStatus::Busy)
    } else {
        None
    }
}

pub(crate) fn sidebar_row_badge(state: &State, page: Page) -> Option<String> {
    match page {
        Page::Jobs => state
            .scoped_jobs
            .in_progress_count()
            .filter(|&count| count > 0)
            .map(|count| count.to_string()),
        Page::Atlas => match &state.cycles.history {
            Load::Ready(history) if !history.rows.is_empty() => {
                Some(history.rows.len().to_string())
            }
            _ => None,
        },
        _ => None,
    }
}
