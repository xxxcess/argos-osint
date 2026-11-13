//! Shared production state and shell; all boundary results are revision fenced.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use fleetworks_contracts::readiness::WorkstationReadiness;
use fleetworks_contracts::search::ActorSearchQuery;
use fleetworks_contracts::Client;
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
use rune_kit::shell::SidebarRow;
use rune_kit::shell::SplitView;
use rune_kit::shell::Toolbar;
use rune_kit::Button;
use rune_kit::ButtonVariant;

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

    /// Concise contextual helper displayed in the sidebar footer for the active view.
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
    PairingConnect,
    PairingRestart,
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
    readiness_request: Option<RequestTask>,
    pub(crate) search_request: Option<RequestTask>,
    pub(crate) dossier_request: Option<RequestTask>,
    pub(crate) readiness_revision: u64,
    pub(crate) readiness_refresh_at: f64,
}

impl Default for State {
    fn default() -> Self {
        Self {
            accessibility: Accessibility::default(),
            page: Page::Workstation,
            setup_gateway: false,
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
            readiness_request: None,
            search_request: None,
            dossier_request: None,
            readiness_revision: 0,
            readiness_refresh_at: 0.0,
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

    /// Isolate data immediately on destination/auth changes and reject old callbacks.
    pub fn disconnect(&mut self) {
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
        self.host_metrics = Load::Idle;
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
                self.ensure_registration();
                self.refresh_readiness();
            }
            Err(error) => self.connection.status = ConnectionStatus::Failed(error),
        }
    }

    pub fn refresh_readiness(&mut self) {
        if !self.connected() {
            self.readiness =
                Load::Failed("Connect to a compatible gateway to refresh checks".into());
            return;
        }
        self.readiness_revision = self.readiness_revision.wrapping_add(1);
        self.readiness_refresh_at = self.clock + 4.0;
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
        self.page == Page::Seed
            && !self.setup_gateway
            && self.connected()
            && self.scoped_logs.window_focused
            && self.readiness_request.is_none()
            && now >= self.readiness_refresh_at
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

fn poll(state: &mut State, now: f64, tx: &mut Tx) -> bool {
    state.clock = now;
    let mut changed = false;
    changed |= state.poll_scoped_logs();
    changed |= state.sync_jobs_binding();
    if state.page == Page::Seed
        && !state.setup_gateway
        && state.connected()
        && state.scoped_logs.window_focused
    {
        if state.maintenance_checks_due(now) {
            state.refresh_readiness();
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
        ..AppSpec::new("Fleetworks", (1180.0, 780.0), State::default, view)
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

    let connection_tone = match &state.connection.status {
        ConnectionStatus::Connected => StatusTone::Ok,
        ConnectionStatus::Connecting => StatusTone::Warn,
        ConnectionStatus::Failed(_) => StatusTone::Err,
        ConnectionStatus::Disconnected => StatusTone::Neutral,
    };
    let pointer_ref = (!preferences.matte()).then_some(&state.pointer_offset);

    let mut sidebar = Sidebar::new()
        .label("Fleetworks pages")
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
            Button::new(state.connection.deployment_label().to_owned())
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
            SidebarRow::new(Page::Workstation.title())
                .icon(Page::Workstation.icon())
                .selected(!state.setup_gateway && state.page == Page::Workstation)
                .on_select(|state: &mut State, _| state.navigate(Page::Workstation))
                .build(&kit)
                .key("nav-workstation"),
        );

    if state.connected() {
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

    let scope = if state.page.is_backend() {
        "Backend-wide".to_owned()
    } else if state.registration.context.is_some() {
        "Workstation activity".to_owned()
    } else {
        "Workstation activity: registration required".to_owned()
    };
    let toolbar = Toolbar::new()
        .title(if state.setup_gateway {
            "Setup Gateway"
        } else if !state.connected() {
            Page::Workstation.title()
        } else {
            state.page.title()
        })
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

    let mut main = rune::col().key("page-content").w_full().h_full().min_h(0.0);
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
            scope_bar = scope_bar
                .child(
                    rune::text(scope)
                        .key("resource-scope")
                        .font(kit.ty.small_medium)
                        .color(kit.c.text_accent),
                )
                .child(
                    rune::text(state.connection.deployment_label().to_owned())
                        .key("deployment")
                        .font(kit.ty.small)
                        .color(kit.c.text_muted),
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
        main = main.child(scope_bar);
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
    let main = main.child(animated_page);

    let base = SplitView::new(main)
        .sidebar(
            &state.sidebar,
            |state: &mut State| &mut state.sidebar,
            sidebar.build(&kit),
        )
        .toolbar(toolbar)
        .titlebar_inset(cx.titlebar_inset())
        .reduced_motion(cx.is_reduced_motion())
        .build(&kit);
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
