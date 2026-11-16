//! Shared production state and shell; all boundary results are revision fenced.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use fleetworks_contracts::ipc::IpcEvent;
use fleetworks_contracts::ipc::ShutdownAck;
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
use rune_kit::Button;
use rune_kit::ButtonVariant;
use rune_kit::ControlSize;
use rune_kit::Dialog;
use rune_kit::IconButton;
use rune_kit::IconButtonVariant;

use crate::accessibility::Accessibility;
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
use crate::material::MetalFinish;
use crate::rebind::Rebind;
use crate::registration::Registration;
use crate::submission::SubmissionKind;
use crate::submission::Submissions;
use crate::theme;
use crate::theme::StatusTone;
use crate::transfer::Transfer;
use crate::transfer::TransferAction;
use crate::ui_components;
use crate::ui_components::ActionButton;
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
    UploadDataset { kind: DatasetKind },
    Datasets,
    Maintenance { operation: MaintenanceOperation },
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
    pub page: Page,
    pub setup_gateway: bool,
    pub backend_kind: BackendKind,
    pub local_backend: LocalBackendState,
    pub(crate) settings_open: bool,
    pub(crate) settings_section: crate::workstation_settings::SettingsSection,
    pub tool_key_dialog: bool,
    pub setup_advanced_open: bool,
    pub workstation_inspector: Option<WorkstationInspector>,
    pub seed_stage: SeedStage,
    pub models_tab: ModelsTab,
    pub models_provider_dialog: bool,
    pub models_role_dialog: bool,
    pub brain_facet: DossierFacet,
    pub selected_readiness: Option<String>,
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
    pub(crate) jobs_refresh_at: f64,
    pub(crate) host_metrics_revision: u64,
    pub(crate) host_metrics_refresh_at: f64,
    pub(crate) atlas_refresh_at: f64,
}

impl Default for State {
    fn default() -> Self {
        Self {
            accessibility: Accessibility::default(),
            page: Page::Workstation,
            setup_gateway: false,
            backend_kind: BackendKind::Remote,
            local_backend: LocalBackendState::default(),
            settings_open: false,
            settings_section: Default::default(),
            tool_key_dialog: false,
            setup_advanced_open: false,
            workstation_inspector: None,
            seed_stage: SeedStage::Seed,
            models_tab: ModelsTab::Roles,
            models_provider_dialog: false,
            models_role_dialog: false,
            brain_facet: DossierFacet::Overview,
            selected_readiness: None,
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
            sidebar: PaneState::new(220.0).range(180.0, 340.0),
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
            clock: 0.0,
            reveal: None,
            selected_workspace: None,
            workspaces: Workspaces::default(),
            registration: Registration::default(),
            rebind: Rebind::default(),
            origin_revision: 0,
            fixture: false,
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
            jobs_refresh_at: 0.0,
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
        self.tool_key_dialog = false;
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

    /// Switch destination mode between Remote Gateway and Local Embedded without leaking tokens or URLs.
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
                self.local_backend.refresh_binary_status();
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
            Some(format!("Stopping local companion backend ({reason})…"));
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
        self.readiness_refresh_at = 0.0;
        self.jobs_refresh_at = 0.0;
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
        self.local_backend.refresh_binary_status();
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
                    "Local companion backend shutdown notice (checkpointed={checkpointed}, relinquished_leases={relinquished_leases})."
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
            "Local IPC event stream skipped {skipped} notifications; refreshing readiness."
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
                "Local companion backend IPC pipe closed unexpectedly (HTTP fallback disabled)."
                    .to_owned()
            } else {
                format!(
                    "Local companion backend IPC pipe closed unexpectedly (HTTP fallback disabled): {stderr}"
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
                    "Local companion backend stopped cleanly (checkpointed={}, relinquished_leases={}).",
                    ack.checkpointed, ack.relinquished_leases
                ));
                self.local_backend.last_shutdown_ack = Some(ack);
            }
            Err(error) => {
                self.local_backend.shutdown_message =
                    Some(format!("Local companion backend shutdown notice: {error}"));
            }
        }
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
        self.readiness = match result {
            Ok(readiness) if readiness.deployment_id == self.connection.deployment_label() => {
                self.readiness_received_at = Some(self.clock);
                Load::Ready(readiness)
            }
            Ok(_) => Load::Failed("Gateway returned readiness for a different deployment".into()),
            Err(error) => Load::Failed(error),
        };
        self.finish_dataset_preview();
    }

    pub fn connected(&self) -> bool {
        self.connection.status == ConnectionStatus::Connected
    }

    pub(crate) fn maintenance_checks_due(&self, now: f64) -> bool {
        if !matches!(self.page, Page::Seed | Page::Workstation)
            || self.setup_gateway
            || !self.connected()
            || !self.scoped_logs.window_focused
            || self.readiness_request.is_some()
            || now < self.readiness_refresh_at
        {
            return false;
        }
        match &self.readiness {
            Load::Ready(readiness) => readiness.checks.iter().any(|check| {
                check.state == ReadinessState::Running || !self.readiness_check_fresh_at(check, now)
            }),
            Load::Idle | Load::Failed(_) => self.page == Page::Seed,
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
        self.jobs_refresh_at = now + JOBS_DISCOVERY_SECS;
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

    pub fn search_key(&mut self, event: &KeyEvent) -> bool {
        match event.key {
            Key::Down => self.brain.move_highlight(true),
            Key::Up => self.brain.move_highlight(false),
            Key::Enter => {
                if let Some(index) = self.brain.highlighted {
                    self.select_actor(index);
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

/// Seconds between discovery reads when no active job is known.
const JOBS_DISCOVERY_SECS: f64 = 5.0;

fn poll(state: &mut State, now: f64, tx: &mut Tx) -> bool {
    state.clock = now;
    let mut changed = false;
    changed |= state.poll_scoped_logs();
    changed |= state.sync_jobs_binding();
    if matches!(state.page, Page::Seed | Page::Workstation)
        && !state.fixture
        && !state.setup_gateway
        && state.connected()
        && state.scoped_logs.window_focused
    {
        if state.maintenance_checks_due(now) {
            state.refresh_readiness();
            changed = true;
        }
        tx.wake_after(Duration::from_secs(2));
    }
    if state.page == Page::Jobs
        && !state.fixture
        && state.connected()
        && state.scoped_logs.window_focused
    {
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
        } else if state.registration.context.is_some() {
            // Nothing known is running. A slow, bounded read still discovers work admitted
            // after this page loaded; the UI selection only narrows what is shown.
            if now >= state.jobs_refresh_at {
                state.discover_registered_jobs(now);
                changed = true;
            }
            tx.wake_after(Duration::from_secs(1));
        }
    }
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
    if event.key == Key::Escape {
        if state.confirm_maintenance_dialog || state.maintenance.confirmation.is_some() {
            let target = if state.page == Page::Workstation {
                "workstation-build-actor-index"
            } else {
                "maintenance-reindex"
            };
            state.cancel_maintenance_confirmation();
            tx.focus(target, false);
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
    let compact_rail = cx
        .layout_size("app-shell")
        .is_some_and(|size| size[0] < 820.0);

    let connection_tone = match &state.connection.status {
        ConnectionStatus::Connected => StatusTone::Ok,
        ConnectionStatus::Connecting => StatusTone::Warn,
        ConnectionStatus::Failed(_) => StatusTone::Err,
        ConnectionStatus::Disconnected => StatusTone::Neutral,
    };
    let pointer_ref = (!preferences.matte()).then_some(&state.pointer_offset);

    let sidebar_el = (!compact_rail).then(|| {
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
                    .child(
                        rune::icon(&icons::BRAND_MARK, 18.0)
                            .color(theme::hex(theme::ACCENT_HOVER))
                            .a11y_hidden(),
                    )
                    .child(
                        rune::text("Fleetworks")
                            .font(kit.ty.body_medium)
                            .color(theme::hex(theme::TEXT_PRIMARY)),
                    )
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
            .child(SidebarHeader::new("Workstation").build(&kit))
            .child(
                SidebarRow::new(Page::Workstation.title())
                    .icon(Page::Workstation.icon())
                    .selected(!state.setup_gateway && state.page == Page::Workstation)
                    .on_select(|state: &mut State, _| state.navigate(Page::Workstation))
                    .build(&kit)
                    .key("nav-workstation"),
            );

        if state.connected() {
            sidebar = sidebar.child(SidebarHeader::new("Operations").build(&kit));
            for page in Page::ALL {
                let mut row_builder = SidebarRow::new(page.title())
                    .icon(page.icon())
                    .selected(!state.setup_gateway && state.page == page)
                    .on_select(move |state: &mut State, _| state.navigate(page));
                if let Some(status) = sidebar_row_status(state, page) {
                    row_builder = row_builder.status(status);
                }
                if let Some(badge) = sidebar_row_badge(state, page) {
                    row_builder = row_builder.badge(badge);
                }
                sidebar = sidebar.child(row_builder.build(&kit).key(format!("nav-{}", page.key())));
            }
        } else {
            sidebar = sidebar.child(
                SidebarRow::new("Setup Gateway")
                    .icon(&icons::KEY)
                    .selected(state.setup_gateway)
                    .on_select(|state: &mut State, _| state.setup_gateway = true)
                    .build(&kit)
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
    let compact_header = cx
        .layout_size("page-content")
        .is_some_and(|size| size[0] < 520.0);
    let title_page = if state.connected() {
        state.page
    } else {
        Page::Workstation
    };
    let toolbar_title = if state.setup_gateway {
        if compact_header {
            "Setup"
        } else {
            "Setup Gateway"
        }
    } else if compact_header {
        title_page.compact_title()
    } else {
        title_page.title()
    };
    let toolbar = Toolbar::new()
        .title(toolbar_title)
        .trailing(crate::material::compact_toolbar(
            ActionButton::new("Setup Gateway")
                .key("connection-editor")
                .icon(&icons::KEY)
                .finish(if state.setup_gateway {
                    MetalFinish::GoldCta
                } else {
                    MetalFinish::Gunmetal
                })
                .on_press(|state: &mut State| {
                    state.setup_gateway = true;
                })
                .build(&kit, preferences, pointer_ref),
            preferences,
        ));

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
        let mut scope_bar = rune::row()
            .w_full()
            .px(20.0)
            .py(10.0)
            .gap(10.0)
            .center_y()
            .fill(theme::hex(theme::SURFACE))
            .border_sides([0.0, 0.0, 1.0, 0.0], theme::hex(theme::BORDER_SUBTLE));
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
        if any_busy {
            scope_bar = scope_bar.child(
                rune::row()
                    .gap(6.0)
                    .center_y()
                    .child(
                        rune_kit::Spinner::new()
                            .size(13.0)
                            .color(theme::hex(theme::ACCENT_HOVER))
                            .build(&kit),
                    )
                    .child(
                        rune::text("Syncing")
                            .font(kit.ty.small_medium)
                            .color(theme::hex(theme::ACCENT_HOVER)),
                    ),
            );
        } else {
            scope_bar = scope_bar.child(ui_components::status_pill(
                match &state.connection.status {
                    ConnectionStatus::Connected => "Online",
                    ConnectionStatus::Connecting => "Connecting",
                    ConnectionStatus::Failed(_) => "Fault",
                    ConnectionStatus::Disconnected => "Offline",
                },
                None,
                connection_tone,
                &kit,
            ));
        }
        content = content.child(scope_bar);
    }

    let screen_key = if state.setup_gateway {
        "screen-setup".to_owned()
    } else if !state.connected() {
        format!("screen-{}", Page::Workstation.key())
    } else {
        format!("screen-{}", state.page.key())
    };
    let page_body = views::page(state, cx, &kit, &mut layers);
    let animated_page = if preferences.reduced_motion {
        rune::col()
            .key(screen_key)
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .child(page_body)
    } else {
        rune::col()
            .key(screen_key)
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .transition(Transition::offset(0.0, 12.0).combined(Transition::opacity()))
            .child(page_body)
    };
    let content = content.child(animated_page);
    let mut main = rune::row().key("main-area").w_full().h_full().min_h(0.0);
    if compact_rail {
        main = main.child(compact_icon_rail(state, &kit, connection_tone, preferences));
    }
    main = main.child(content);
    if state.connected()
        && !state.setup_gateway
        && cx
            .layout_with("main-area", |rect| rect[2] >= 1120.0)
            .unwrap_or(false)
    {
        main = main.child(context_rail(state, &kit, connection_tone));
    }

    if state.settings_open {
        let width = cx
            .layout_size("main-area")
            .map(|size| (size[0] - 24.0).clamp(420.0, 940.0))
            .unwrap_or(760.0);
        Dialog::new("workstation-settings", "Settings")
            .open(true)
            .width(width)
            .fit()
            .click_outside(true)
            .on_close(|state: &mut State| state.settings_open = false)
            .child(crate::workstation_settings::page(state, cx, &kit))
            .build(&kit, &mut layers);
    }

    let mut split = SplitView::new(main)
        .toolbar(toolbar)
        .titlebar_inset(cx.titlebar_inset())
        .reduced_motion(cx.is_reduced_motion());
    if let Some(sidebar_el) = sidebar_el {
        split = split.sidebar(
            &state.sidebar,
            |state: &mut State| &mut state.sidebar,
            sidebar_el,
        );
    }
    let base = split.build(&kit).key("app-shell");
    let base = if state.fixture {
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
            .child(base.grow(1.0).min_h(0.0))
    } else {
        base
    };
    rune_kit::root(base, layers)
        .on_window_focus(State::log_window_focus)
        .on_pointer_move_local(|state: &mut State, pointer, tx| {
            let nx = (pointer.window_x / 1180.0).clamp(0.0, 1.0);
            let ny = (pointer.window_y / 780.0).clamp(0.0, 1.0);
            state
                .pointer_offset
                .set([(nx - 0.5) * 56.0, (ny - 0.5) * 38.0]);
            state
                .shadow_offset
                .set([(0.5 - nx) * 16.0, (0.5 - ny) * 14.0]);
            tx.live_only();
        })
}

fn compact_icon_rail(
    state: &State,
    kit: &rune_kit::Kit,
    connection_tone: StatusTone,
    preferences: crate::accessibility::ViewPreferences,
) -> El {
    let mut rail = rune::col()
        .key("compact-icon-rail")
        .w(52.0)
        .h_full()
        .py(10.0)
        .px(6.0)
        .gap(6.0)
        .items_center()
        .fill(theme::hex(theme::SURFACE))
        .border_sides([0.0, 1.0, 0.0, 0.0], theme::hex(theme::BORDER_SUBTLE))
        .child(
            rune::col()
                .py(4.0)
                .items_center()
                .child(ui_components::status_dot(connection_tone, preferences)),
        )
        .child(
            IconButton::new(Page::Workstation.icon())
                .variant(if !state.setup_gateway && state.page == Page::Workstation {
                    IconButtonVariant::Contrast
                } else {
                    IconButtonVariant::GhostMuted
                })
                .control_size(ControlSize::Large)
                .label(Page::Workstation.title())
                .help(Page::Workstation.title())
                .on_press(|state: &mut State| state.navigate(Page::Workstation))
                .build(kit)
                .key("nav-workstation"),
        );
    if state.connected() {
        for page in Page::ALL {
            rail = rail.child(
                IconButton::new(page.icon())
                    .variant(if !state.setup_gateway && state.page == page {
                        IconButtonVariant::Contrast
                    } else {
                        IconButtonVariant::GhostMuted
                    })
                    .control_size(ControlSize::Large)
                    .label(page.title())
                    .help(page.title())
                    .on_press(move |state: &mut State| state.navigate(page))
                    .build(kit)
                    .key(format!("nav-{}", page.key())),
            );
        }
    } else {
        rail = rail.child(
            IconButton::new(&icons::KEY)
                .variant(if state.setup_gateway {
                    IconButtonVariant::Contrast
                } else {
                    IconButtonVariant::GhostMuted
                })
                .control_size(ControlSize::Large)
                .label("Setup Gateway")
                .help("Setup Gateway")
                .on_press(|state: &mut State| state.setup_gateway = true)
                .build(kit)
                .key("nav-setup"),
        );
    }
    rail.child(rune::spacer()).child(
        IconButton::new(&icons::WORKSTATION)
            .variant(if state.settings_open {
                IconButtonVariant::Contrast
            } else {
                IconButtonVariant::GhostMuted
            })
            .control_size(ControlSize::Large)
            .label("Settings")
            .help("Settings")
            .on_press(|state: &mut State| state.settings_open = true)
            .build(kit)
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
        let records = if count_num == 1 {
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

fn sidebar_row_badge(state: &State, page: Page) -> Option<String> {
    match page {
        Page::Jobs => state
            .scoped_jobs
            .page
            .as_ref()
            .filter(|page| !page.rows.is_empty())
            .map(|page| page.rows.len().to_string()),
        Page::Atlas => match &state.cycles.history {
            Load::Ready(history) if !history.rows.is_empty() => {
                Some(history.rows.len().to_string())
            }
            _ => None,
        },
        _ => None,
    }
}
