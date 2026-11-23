//! Runtime-neutral Inbox QA state and views. The selected registered backend owns admission,
//! model execution, evidence, and the authoritative latest exchange.

use std::time::Duration;

use fleetworks_contracts::chat::ChatSourceKind;
use fleetworks_contracts::chat::ChatSourceReference;
use fleetworks_contracts::chat::QaAvailabilityReason;
use fleetworks_contracts::chat::QaCancelRequest;
use fleetworks_contracts::chat::QaEventPage;
use fleetworks_contracts::chat::QaEventQuery;
use fleetworks_contracts::chat::QaPhase;
use fleetworks_contracts::chat::QaSettings;
use fleetworks_contracts::chat::QaSettingsUpdate;
use fleetworks_contracts::chat::QaTurn;
use fleetworks_contracts::chat::QaTurnNotice;
use fleetworks_contracts::chat::QaTurnRequest;
use fleetworks_contracts::chat::QaTurnState;
use fleetworks_contracts::chat::MAX_EVENT_WAIT_SECS;
use fleetworks_contracts::chat::MAX_INPUT_BYTES;
use fleetworks_contracts::context::MutationContext;
use fleetworks_contracts::context::Permission;
use fleetworks_contracts::context::RegisteredContext;
use fleetworks_contracts::Client;
use fleetworks_contracts::ClientError;
use fleetworks_workstation::native::BackendKind;
use rune::El;
use rune::Key;
use rune::KeyEvent;
use rune::LiveRegion;
use rune::TextEdit;
use rune::Tx;
use rune_kit::Kit;

use crate::accessibility::ViewPreferences;
use crate::assistant;
use crate::chat_ui;
use crate::connection::GatewayEffects;
use crate::connection::RequestTask;
use crate::material::{self, GlassSurface, MetalFinish};
use crate::theme;
use crate::theme::StatusTone;
use crate::ui_components::ActionButton;
use crate::State;

const SETTINGS_REFRESH: f64 = 5.0;
const MIN_EVENT_INTERVAL: f64 = 0.2;
const COMPOSER: &str = "inbox-qa-composer";
const SOURCES_TOGGLE: &str = "inbox-qa-sources-toggle";
const SOURCES_CONTENT: &str = "inbox-qa-sources";
const SETTINGS_CONTROL: &str = "inbox-qa-toggle";
#[cfg(test)]
const REPLY: &str = "chat-message-inbox-qa-answer";
const DETAILS_TOGGLE: &str = "inbox-qa-availability-toggle";
const DETAILS: &str = "inbox-qa-availability-details";

struct Binding {
    kind: BackendKind,
    origin: String,
    context: RegisteredContext,
}

impl Binding {
    fn matches_target(&self, state: &State) -> bool {
        self.kind == state.backend_kind && self.origin == state.connection.origin.text
    }

    fn matches(&self, state: &State) -> bool {
        self.matches_target(state)
            && state
                .registration
                .context
                .as_ref()
                .is_some_and(|context| self.context.same_request_context(context))
    }
}

struct Pending {
    request: QaTurnRequest,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum Kind {
    Settings,
    Update,
    Submit,
    Latest,
    Events,
    Cancel,
}

impl Kind {
    fn changes_backend(self) -> bool {
        matches!(self, Self::Update | Self::Submit | Self::Cancel)
    }
}

enum Request {
    Settings,
    Update(QaSettingsUpdate),
    Submit(QaTurnRequest),
    Latest,
    Events(QaEventQuery),
    Cancel(QaCancelRequest),
}

impl Request {
    fn kind(&self) -> Kind {
        match self {
            Self::Settings => Kind::Settings,
            Self::Update(_) => Kind::Update,
            Self::Submit(_) => Kind::Submit,
            Self::Latest => Kind::Latest,
            Self::Events(_) => Kind::Events,
            Self::Cancel(_) => Kind::Cancel,
        }
    }
}

enum Reply {
    Settings(Result<QaSettings, ClientError>),
    Update(Result<QaSettings, ClientError>),
    Submit(Result<QaTurn, ClientError>),
    Latest(Result<Option<QaTurn>, ClientError>),
    Events(Result<QaEventPage, ClientError>),
    Cancel(Result<QaTurn, ClientError>),
}

struct InFlight {
    kind: Kind,
    _task: RequestTask,
}

#[derive(Clone, Copy)]
struct Ticket {
    epoch: u64,
    origin_revision: u64,
    request_revision: u64,
}

pub(crate) struct InboxQa {
    binding: Option<Binding>,
    settings: Option<QaSettings>,
    latest: Option<QaTurn>,
    pending: Option<Pending>,
    draft: TextEdit,
    source: Option<assistant::Context>,
    status: Option<&'static str>,
    settings_loaded: bool,
    details_open: bool,
    sources_open: bool,
    startup_seen: bool,
    change_to: Option<bool>,
    request: Option<InFlight>,
    request_revision: u64,
    resume: bool,
    next_settings_at: f64,
    next_turn_at: f64,
    retry_count: u32,
}

impl Default for InboxQa {
    fn default() -> Self {
        Self {
            binding: None,
            settings: None,
            latest: None,
            pending: None,
            draft: TextEdit::new(""),
            source: None,
            status: None,
            settings_loaded: false,
            details_open: false,
            sources_open: false,
            startup_seen: false,
            change_to: None,
            request: None,
            request_revision: 0,
            resume: true,
            next_settings_at: 0.0,
            next_turn_at: 0.0,
            retry_count: 0,
        }
    }
}

impl InboxQa {
    /// An ordinary disconnect hides the exchange until the same authority is confirmed again.
    pub(crate) fn detach(&mut self) {
        self.request = None;
        self.request_revision = self.request_revision.wrapping_add(1);
        self.settings_loaded = false;
        self.request_resume();
    }

    fn retire(&mut self) {
        self.latest = None;
        self.sources_open = false;
        self.pending = None;
        self.draft = TextEdit::new("");
        self.source = None;
    }

    pub(crate) fn request_resume(&mut self) {
        self.resume = true;
        self.next_turn_at = 0.0;
        self.next_settings_at = 0.0;
        self.retry_count = 0;
    }

    fn active(&self) -> Option<&QaTurn> {
        self.latest.as_ref().filter(|turn| active(turn.state))
    }

    fn mutation_in_flight(&self) -> bool {
        self.request
            .as_ref()
            .is_some_and(|request| request.kind.changes_backend())
    }

    fn adopt(&mut self, turn: Option<QaTurn>) {
        if let (Some(old), Some(next)) = (&self.latest, &turn) {
            if old.turn_id == next.turn_id && next.cursor < old.cursor {
                return;
            }
        }
        if self.latest.as_ref().map(|turn| &turn.turn_id) != turn.as_ref().map(|turn| &turn.turn_id)
        {
            self.sources_open = false;
        }
        self.latest = turn;
    }

    fn apply_settings(&mut self, settings: QaSettings, now: f64) {
        let generation_changed = self
            .settings
            .as_ref()
            .is_some_and(|old| settings.generation != old.generation);
        if !settings.enabled || generation_changed {
            self.retire();
            self.resume = true;
        }
        if self.settings.and_then(|old| old.availability_reason) != settings.availability_reason {
            self.details_open = false;
        }
        self.settings = Some(settings);
        self.settings_loaded = true;
        self.change_to = None;
        self.next_settings_at = now + SETTINGS_REFRESH;
    }

    fn accept_pending(&mut self, turn: &QaTurn) -> bool {
        if self
            .pending
            .as_ref()
            .is_none_or(|pending| pending.request.request_id != turn.request_id)
        {
            return false;
        }
        self.pending = None;
        self.draft = TextEdit::new("");
        self.status = None;
        self.resume = false;
        true
    }

    fn retry_after(&mut self, now: f64) -> f64 {
        self.retry_count = (self.retry_count + 1).min(5);
        let delay = (0.25 * f64::from(1 << self.retry_count)).min(SETTINGS_REFRESH);
        let jitter = f64::from((self.request_revision % 10) as u32) * 0.022;
        now + delay + jitter
    }
}

fn active(state: QaTurnState) -> bool {
    matches!(state, QaTurnState::Queued | QaTurnState::Running)
}

fn denied(error: ClientError) -> bool {
    matches!(error, ClientError::Status(401 | 403))
}

fn admission_rejected(error: ClientError) -> bool {
    matches!(
        error,
        ClientError::Status(400..=407 | 409..=499 | 503)
            | ClientError::InvalidRequest
            | ClientError::RequestTooLarge
    )
}

fn status_message(error: ClientError) -> &'static str {
    match error {
        ClientError::Status(401 | 403) => "This registration is no longer authorized.",
        ClientError::Status(409) => "The backend state changed. Check the current state and retry.",
        ClientError::Status(429) => "Backend inference is busy. Try again shortly.",
        ClientError::Status(503) => "Backend inference is unavailable.",
        ClientError::Transport | ClientError::Timeout => "The selected backend did not respond.",
        _ => "Backend inference could not complete the request.",
    }
}

impl State {
    /// Install synthetic QA data for an effects-disabled native fixture.
    #[doc(hidden)]
    pub fn seed_inbox_qa_fixture(&mut self, settings: QaSettings, turn: Option<QaTurn>) {
        if !self.fixture {
            return;
        }
        if let Some(capabilities) = &mut self.connection.capabilities {
            if !capabilities
                .features
                .iter()
                .any(|feature| feature == "inbox-qa-v2")
            {
                capabilities.features.push("inbox-qa-v2".into());
            }
        }
        self.inbox_qa.apply_settings(settings, self.clock);
        self.inbox_qa.adopt(turn);
        self.inbox_qa.resume = false;
    }

    /// Open the synthetic Connection settings with QA availability details expanded.
    #[doc(hidden)]
    pub fn fixture_show_gateway_qa_details(&mut self) {
        if self.fixture {
            self.inbox.close();
            self.open_connection_settings();
            self.inbox_qa.details_open = true;
        }
    }

    pub(crate) fn supports_inbox_qa(&self) -> bool {
        self.connected()
            && self.registration.context.is_some()
            && self.permission(Permission::ReadBackend)
            && self
                .connection
                .capabilities
                .as_ref()
                .is_some_and(|capabilities| {
                    capabilities
                        .features
                        .iter()
                        .any(|feature| feature == "inbox-qa-v2")
                })
    }

    fn inbox_qa_settings(&self) -> Option<&QaSettings> {
        (self.supports_inbox_qa()
            && self.inbox_qa.settings_loaded
            && (self.fixture
                || self
                    .inbox_qa
                    .binding
                    .as_ref()
                    .is_some_and(|binding| binding.matches(self))))
        .then_some(self.inbox_qa.settings.as_ref())
        .flatten()
    }

    fn can_manage_inbox_qa(&self) -> bool {
        self.permission(Permission::ManageConfiguration)
            && self
                .inbox_qa_settings()
                .is_some_and(|settings| settings.can_manage)
    }

    fn inbox_qa_visible(&self) -> bool {
        self.permission(Permission::ReadWorkstationActivity)
            && self.inbox_qa.change_to != Some(false)
            && self.inbox_qa_settings().is_some_and(|settings| {
                settings.enabled && !matches!(settings.phase, QaPhase::Off | QaPhase::Stopping)
            })
    }

    fn can_send_inbox_qa(&self) -> bool {
        self.inbox_qa_visible()
            && self.permission(Permission::SubmitInteractive)
            && self
                .inbox_qa_settings()
                .is_some_and(|settings| settings.phase == QaPhase::Ready)
            && !self.inbox_qa.resume
            && self.inbox_qa.pending.is_none()
            && self.inbox_qa.active().is_none()
            && !self.inbox_qa.mutation_in_flight()
            && self.inbox_qa.change_to.is_none()
    }

    fn sync_inbox_qa_binding(&mut self) -> bool {
        if self.fixture {
            return false;
        }
        let mut changed = false;
        if self.inbox_qa.binding.as_ref().is_some_and(|binding| {
            !binding.matches_target(self)
                || (self.connected()
                    && self.registration.context.is_some()
                    && (!binding.matches(self) || !self.supports_inbox_qa()))
        }) {
            self.inbox_qa = InboxQa {
                request_revision: self.inbox_qa.request_revision.wrapping_add(1),
                ..InboxQa::default()
            };
            changed = true;
        }
        if self.inbox_qa.binding.is_none() && self.supports_inbox_qa() {
            if let Some(context) = self.registration.context.clone() {
                self.inbox_qa.binding = Some(Binding {
                    kind: self.backend_kind,
                    origin: self.connection.origin.text.clone(),
                    context,
                });
                return true;
            }
        }
        changed
    }

    pub(crate) fn poll_inbox_qa(&mut self, now: f64, tx: &mut Tx) -> bool {
        let changed = self.sync_inbox_qa_binding();
        if self.fixture || !self.supports_inbox_qa() || self.inbox_qa.request.is_some() {
            return changed;
        }
        let qa = &self.inbox_qa;
        let watch_settings = !qa.settings_loaded
            || qa.change_to.is_some()
            || self.inbox.open
            || self.settings_open
            || self.startup.data_setup_active()
            || qa.pending.is_some()
            || qa.active().is_some();
        if watch_settings && now >= qa.next_settings_at {
            return self.dispatch_inbox_qa(Request::Settings) || changed;
        }
        if watch_settings {
            tx.wake_after(Duration::from_secs_f64(qa.next_settings_at - now));
        }
        if !self.inbox_qa_visible()
            || (!self.inbox.open && qa.active().is_none() && qa.pending.is_none())
        {
            return changed;
        }
        if now < qa.next_turn_at {
            tx.wake_after(Duration::from_secs_f64(qa.next_turn_at - now));
            return changed;
        }
        let request = if qa.resume {
            Some(Request::Latest)
        } else if let Some(pending) = &qa.pending {
            Some(Request::Submit(pending.request.clone()))
        } else {
            qa.active().map(|turn| {
                Request::Events(QaEventQuery::new(
                    turn.turn_id.clone(),
                    turn.cursor,
                    MAX_EVENT_WAIT_SECS,
                ))
            })
        };
        request.is_some_and(|request| self.dispatch_inbox_qa(request)) || changed
    }

    fn dispatch_inbox_qa(&mut self, request: Request) -> bool {
        let (Some(services), Some(client), Some(context)) =
            (&self.services, &self.client, &self.registration.context)
        else {
            return false;
        };
        let Ok(client) = client.with_registered_context(context) else {
            return false;
        };
        self.inbox_qa.request_revision = self.inbox_qa.request_revision.wrapping_add(1);
        let ticket = Ticket {
            epoch: self.connection.epoch,
            origin_revision: self.origin_revision,
            request_revision: self.inbox_qa.request_revision,
        };
        let kind = request.kind();
        let Some(task) = services.inbox_qa(client, request, ticket) else {
            return false;
        };
        self.inbox_qa.request = Some(InFlight { kind, _task: task });
        true
    }

    fn receive_inbox_qa(&mut self, ticket: Ticket, reply: Reply, tx: &mut Tx) {
        if self.connection.epoch != ticket.epoch
            || self.origin_revision != ticket.origin_revision
            || self.inbox_qa.request_revision != ticket.request_revision
            || !self
                .inbox_qa
                .binding
                .as_ref()
                .is_some_and(|binding| binding.matches(self))
        {
            return;
        }
        let now = tx.clock();
        let qa = &mut self.inbox_qa;
        qa.request = None;
        match reply {
            Reply::Settings(Ok(settings)) => {
                if !qa.settings_loaded {
                    qa.status = None;
                }
                qa.apply_settings(settings, now);
            }
            Reply::Update(Ok(settings)) => {
                qa.apply_settings(settings, now);
                qa.status = None;
            }
            Reply::Settings(Err(error)) | Reply::Update(Err(error)) => {
                if admission_rejected(error) {
                    qa.change_to = None;
                }
                qa.status = Some(status_message(error));
                if denied(error) {
                    qa.retire();
                    qa.settings_loaded = false;
                }
                qa.next_settings_at = qa.retry_after(now);
            }
            Reply::Submit(Ok(turn)) => {
                if qa.accept_pending(&turn) {
                    qa.adopt(Some(turn));
                    qa.retry_count = 0;
                    if self.inbox.open && self.inbox.at_bottom {
                        tx.focus(COMPOSER, tx.focus_visible());
                    }
                } else {
                    qa.resume = true;
                    qa.next_turn_at = qa.retry_after(now);
                }
            }
            Reply::Submit(Err(error)) if admission_rejected(error) => {
                qa.pending = None;
                qa.status = Some(status_message(error));
                qa.next_settings_at = now;
                if denied(error) {
                    qa.retire();
                    qa.settings_loaded = false;
                } else if self.inbox.open && self.inbox.at_bottom {
                    tx.focus(COMPOSER, tx.focus_visible());
                }
            }
            Reply::Submit(Err(_)) => {
                qa.status = Some("Checking whether the backend accepted this question…");
                qa.resume = true;
                qa.next_turn_at = qa.retry_after(now);
            }
            Reply::Latest(Ok(turn)) => {
                qa.resume = false;
                qa.retry_count = 0;
                let had_exchange = qa.latest.is_some() || qa.pending.is_some();
                let accepted_pending = turn.as_ref().is_some_and(|turn| qa.accept_pending(turn));
                if qa.pending.is_none() {
                    qa.adopt(turn);
                }
                if self.inbox.open && self.inbox.at_bottom && qa.latest.is_some() {
                    if !had_exchange {
                        tx.scroll_to_bottom_instant(crate::inbox::SCROLL);
                    }
                    if accepted_pending {
                        tx.focus(COMPOSER, tx.focus_visible());
                    }
                }
            }
            Reply::Events(Ok(page)) => {
                if qa
                    .latest
                    .as_ref()
                    .is_some_and(|turn| turn.turn_id == page.turn.turn_id)
                {
                    qa.adopt(Some(page.turn));
                    qa.status = None;
                } else {
                    qa.resume = true;
                }
                qa.retry_count = 0;
                qa.next_turn_at = now + MIN_EVENT_INTERVAL;
            }
            Reply::Cancel(Ok(turn)) => {
                qa.adopt(Some(turn));
                qa.status = None;
                if self.inbox.open && self.inbox.at_bottom {
                    tx.focus(COMPOSER, true);
                }
            }
            Reply::Latest(Err(error)) | Reply::Events(Err(error)) | Reply::Cancel(Err(error)) => {
                qa.status = Some(status_message(error));
                qa.resume = true;
                qa.next_turn_at = qa.retry_after(now);
                if denied(error) {
                    qa.retire();
                    qa.settings_loaded = false;
                }
            }
        }
    }

    fn set_inbox_inference(&mut self, enabled: bool) {
        if !self.can_manage_inbox_qa()
            || self.inbox_qa.change_to.is_some()
            || self
                .inbox_qa
                .request
                .as_ref()
                .is_some_and(|task| task.kind == Kind::Update)
        {
            return;
        }
        let (Some(context), Some(settings)) =
            (&self.registration.context, self.inbox_qa_settings())
        else {
            return;
        };
        if settings.enabled == enabled || settings.phase == QaPhase::Stopping {
            return;
        }
        let update =
            QaSettingsUpdate::new(MutationContext::new(context), settings.revision, enabled);
        if self.dispatch_inbox_qa(Request::Update(update)) {
            self.inbox_qa.change_to = Some(enabled);
            self.inbox_qa.status = None;
        }
    }

    fn send_inbox_question(&mut self, tx: &mut Tx) {
        if !self.can_send_inbox_qa() || self.inbox_qa.draft.is_composing() {
            return;
        }
        let question = self.inbox_qa.draft.text.trim();
        if question.is_empty() {
            return;
        }
        if question.len() > MAX_INPUT_BYTES {
            self.inbox_qa.status = Some("Shorten the question to 4 KiB or fewer.");
            return;
        }
        let Some(context) = self.registration.context.as_ref() else {
            return;
        };
        let Ok(request_id) = crate::platform::request_id() else {
            self.inbox_qa.status = Some("Could not prepare the question. Try again.");
            return;
        };
        let source = self.inbox_qa.source.as_ref().and_then(optional_source);
        let Ok(request) = QaTurnRequest::new(
            request_id,
            MutationContext::new(context),
            question.to_owned(),
            source,
        ) else {
            self.inbox_qa.status = Some("Check the question and selected source, then try again.");
            return;
        };
        if self.dispatch_inbox_qa(Request::Submit(request.clone())) {
            self.inbox_qa.pending = Some(Pending { request });
            self.inbox_qa.status = None;
            tx.scroll_to_bottom_instant(crate::inbox::SCROLL);
        }
    }

    fn stop_inbox_question(&mut self) {
        if !self.inbox_qa_visible()
            || !self.permission(Permission::SubmitInteractive)
            || self.inbox_qa.mutation_in_flight()
        {
            return;
        }
        let (Some(context), Some(turn)) = (&self.registration.context, self.inbox_qa.active())
        else {
            return;
        };
        let request = QaCancelRequest::new(MutationContext::new(context), turn.turn_id.clone());
        if self.dispatch_inbox_qa(Request::Cancel(request)) {
            self.inbox_qa.status = None;
        }
    }
}

impl GatewayEffects {
    fn inbox_qa(&self, client: Client, request: Request, ticket: Ticket) -> Option<RequestTask> {
        let ui = self.ui.get()?.clone();
        let task = self.runtime.spawn(async move {
            let reply = match request {
                Request::Settings => Reply::Settings(client.qa_settings().await),
                Request::Update(request) => {
                    Reply::Update(client.update_qa_settings(&request).await)
                }
                Request::Submit(request) => Reply::Submit(client.submit_qa_turn(&request).await),
                Request::Latest => Reply::Latest(client.latest_qa_turn().await),
                Request::Events(request) => Reply::Events(client.qa_events(&request).await),
                Request::Cancel(request) => Reply::Cancel(client.cancel_qa_turn(&request).await),
            };
            ui.update(move |state, tx| state.receive_inbox_qa(ticket, reply, tx));
        });
        Some(RequestTask(task.abort_handle()))
    }
}

fn target_label(state: &State) -> &'static str {
    if state.is_local_backend() {
        "Runs on this device"
    } else {
        "Runs on the connected gateway"
    }
}

fn phase_label(state: &State) -> (&'static str, StatusTone) {
    if state.inbox_qa.change_to == Some(false) {
        return (stopping_label(state), StatusTone::Warn);
    }
    if state.inbox_qa.change_to == Some(true) {
        return ("Starting", StatusTone::Info);
    }
    match state.inbox_qa_settings().map(|settings| settings.phase) {
        Some(QaPhase::Off) => ("Off", StatusTone::Neutral),
        Some(QaPhase::Starting) => ("Starting", StatusTone::Info),
        Some(QaPhase::Ready) => ("Ready", StatusTone::Ok),
        Some(QaPhase::Stopping) => (stopping_label(state), StatusTone::Warn),
        Some(QaPhase::Unavailable) => ("Unavailable", StatusTone::Warn),
        _ if state.supports_inbox_qa() => ("Checking", StatusTone::Info),
        _ => ("Unavailable", StatusTone::Neutral),
    }
}

fn stopping_label(state: &State) -> &'static str {
    if state.is_local_backend() {
        "Stopping"
    } else {
        "Stopping · replica unconfirmed"
    }
}

fn hint(text: &str, kit: &Kit) -> El {
    rune::text(text)
        .font(kit.ty.small)
        .color(theme::hex(theme::TEXT_MUTED))
        .w_full()
        .wrap_anywhere()
}

struct EmbeddedProblem {
    status: &'static str,
    details: &'static str,
}

fn embedded_problem(state: &State) -> Option<EmbeddedProblem> {
    if !state.is_local_backend() || state.inbox_qa.change_to.is_some() {
        return None;
    }
    let settings = state.inbox_qa_settings()?;
    if settings.phase != QaPhase::Unavailable {
        return None;
    }
    let problem = match settings.availability_reason? {
        QaAvailabilityReason::PackMissing => EmbeddedProblem {
            status: "Embedded QA pack is missing",
            details: "Install the optional Inbox QA pack for this Fleetworks release and platform.",
        },
        QaAvailabilityReason::PackInvalid => EmbeddedProblem {
            status: "Embedded QA pack could not be verified",
            details: "Reinstall the Inbox QA pack for this Fleetworks release and platform.",
        },
        QaAvailabilityReason::SandboxUnsupported => EmbeddedProblem {
            status: "Embedded inference isn’t supported on this host",
            details: "Embedded inference on Linux requires Landlock ABI 3 or newer.",
        },
        QaAvailabilityReason::WorkerStartFailed => EmbeddedProblem {
            status: "Embedded inference worker could not start",
            details: "Check the Embedded companion logs for the worker startup error.",
        },
        _ => return None,
    };
    Some(problem)
}

fn problem_content(state: &State, problem: EmbeddedProblem, kit: &Kit) -> El {
    let expanded = state.inbox_qa.details_open;
    let toggle = rune_kit::Button::new("Details")
        .variant(rune_kit::ButtonVariant::Ghost)
        .on_press_tx(|state: &mut State, tx| {
            state.inbox_qa.details_open = !state.inbox_qa.details_open;
            tx.keep_on_screen(DETAILS_TOGGLE);
        })
        .build(kit)
        .key(DETAILS_TOGGLE)
        .label("Inference availability details")
        .expanded(expanded);
    rune::col()
        .w_full()
        .min_w(0.0)
        .gap(4.0)
        .child(
            hint(problem.status, kit)
                .color(StatusTone::Warn.fg())
                .font(kit.ty.small_medium)
                .role("status"),
        )
        .child(rune::row().child(toggle))
        .children(expanded.then(|| hint(problem.details, kit).key(DETAILS)))
}

pub(crate) fn settings_content(state: &State, kit: &Kit, preferences: ViewPreferences) -> El {
    let (phase, tone) = phase_label(state);
    let problem = embedded_problem(state);
    let mut content = rune::col()
        .w_full()
        .min_w(0.0)
        .gap(8.0)
        .child(hint(target_label(state), kit));
    let mut actions = rune::row()
        .w_full()
        .flex_wrap()
        .center_y()
        .gap(8.0)
        .children(
            problem
                .is_none()
                .then(|| crate::ui_components::status_pill(phase, None, tone, kit)),
        );
    if let Some(settings) = state.inbox_qa_settings() {
        if state.can_manage_inbox_qa() {
            let enable = !settings.enabled;
            actions = actions.child(
                ActionButton::new(if enable { "Turn On" } else { "Turn Off" })
                    .key(SETTINGS_CONTROL)
                    .finish(MetalFinish::Gunmetal)
                    .disabled(
                        state.inbox_qa.change_to.is_some() || settings.phase == QaPhase::Stopping,
                    )
                    .on_press(move |state: &mut State| state.set_inbox_inference(enable))
                    .build(kit, preferences, None),
            );
        } else if !settings.enabled && problem.is_none() {
            content = content.child(hint(operator_nudge(state), kit));
        }
    } else if !state.supports_inbox_qa() {
        content = content.child(hint(
            "Backend inference is unavailable for this selection.",
            kit,
        ));
    }
    if let Some(problem) = problem {
        content = content.child(problem_content(state, problem, kit));
    } else if let Some(status) = state.inbox_qa.status {
        content = content.child(hint(status, kit).role("status"));
    } else if phase == "Unavailable" && state.supports_inbox_qa() {
        content = content.child(hint(unavailable_nudge(state), kit));
    }
    content.child(actions)
}

fn operator_nudge(state: &State) -> &'static str {
    if state.is_local_backend() {
        "Ask an authorized workstation manager to enable backend inference."
    } else {
        "Ask your gateway operator to enable backend inference."
    }
}

fn unavailable_nudge(state: &State) -> &'static str {
    if state.is_local_backend() {
        "Embedded inference is unavailable. Check Connection settings for backend status."
    } else {
        "Ask your gateway operator to check the inference worker and model pack."
    }
}

pub(crate) fn setup_choice(state: &State) -> bool {
    state.supports_inbox_qa() && !state.inbox_qa.startup_seen
}

pub(crate) fn continue_setup(state: &mut State, tx: &mut Tx) {
    state.inbox_qa.startup_seen = true;
    tx.focus("startup-data-stage", true);
}

fn optional_source(context: &assistant::Context) -> Option<ChatSourceReference> {
    assistant::source_for(context)
        .filter(|source| matches!(source.kind, ChatSourceKind::Job | ChatSourceKind::Cycle))
}

fn citation_kind(kind: ChatSourceKind) -> &'static str {
    match kind {
        ChatSourceKind::Job => "Job",
        ChatSourceKind::Cycle => "Cycle",
        ChatSourceKind::Readiness => "Readiness",
        ChatSourceKind::HostMetric => "Host metric",
        ChatSourceKind::JobCount => "Job count",
        _ => "Evidence",
    }
}

fn answer_state(turn: &QaTurn) -> (chat_ui::TurnState, &'static str) {
    use chat_ui::TurnState;
    let (stage, fallback) = match turn.state {
        QaTurnState::Queued => (TurnState::Queued, ""),
        QaTurnState::Running => (TurnState::Running, ""),
        QaTurnState::Answered => (TurnState::Final, ""),
        QaTurnState::InsufficientContext => (
            TurnState::Final,
            "The backend could not verify an answer from current evidence.",
        ),
        QaTurnState::Canceled => (TurnState::Final, "Stopped"),
        _ => (
            TurnState::Failed {
                reason: "Backend inference could not finish".into(),
            },
            "",
        ),
    };
    let detail = match turn.notice {
        Some(QaTurnNotice::Greeting) => {
            "Hi! Try asking if actor search is ready, how much memory is in use, or which jobs are running."
        }
        Some(QaTurnNotice::OutOfScope) => {
            "I can answer questions about this workstation’s health, readiness, resource usage, and jobs."
        }
        Some(QaTurnNotice::ObservationUnavailable) => {
            "I couldn’t obtain the relevant current measurement for that question."
        }
        _ => fallback,
    };
    (stage, detail)
}

fn answer_sources(turn_id: &str, citations: &[chat_ui::Citation], expanded: bool, kit: &Kit) -> El {
    let turn_id = turn_id.to_owned();
    let trigger = rune_kit::Button::new(format!("Sources ({})", citations.len()))
        .variant(rune_kit::ButtonVariant::Ghost)
        .icon(if expanded {
            kit.glyph.triangle_down
        } else {
            kit.glyph.triangle_right
        })
        .on_press_tx(move |state: &mut State, tx| {
            let qa = &mut state.inbox_qa;
            if qa.pending.is_none()
                && qa
                    .latest
                    .as_ref()
                    .is_some_and(|turn| turn.turn_id == turn_id)
            {
                qa.sources_open = !qa.sources_open;
                if !state.inbox.at_bottom {
                    tx.keep_on_screen(SOURCES_TOGGLE);
                }
            }
        })
        .build(kit)
        .key(SOURCES_TOGGLE)
        .expanded(expanded);
    rune::col()
        .w_full()
        .min_w(0.0)
        .gap(4.0)
        .child(rune::row().child(trigger))
        .children(expanded.then(|| {
            chat_ui::citation_rows(citations, kit)
                .key(SOURCES_CONTENT)
                .role("region")
                .label("Source citations")
        }))
}

pub(crate) fn exchange(state: &State, kit: &Kit) -> Option<El> {
    if !state.inbox_qa_visible() {
        return None;
    }
    let qa = &state.inbox_qa;
    let (question, stage, detail, findings, turn_id) = if let Some(pending) = &qa.pending {
        (
            pending.request.question.as_str(),
            chat_ui::TurnState::Queued,
            "",
            &[][..],
            None,
        )
    } else if let Some(turn) = &qa.latest {
        let (stage, detail) = answer_state(turn);
        (
            turn.question.as_str(),
            stage,
            detail,
            turn.findings.as_slice(),
            Some(turn.turn_id.as_str()),
        )
    } else {
        return None;
    };
    let question = chat_ui::ChatMessage {
        id: "inbox-qa-question".into(),
        role: chat_ui::Role::Operator,
        state: chat_ui::TurnState::Final,
        text: question.to_owned(),
        citations: Vec::new(),
    };
    let mut answer = findings
        .iter()
        .map(|finding| finding.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    if !detail.is_empty() {
        if !answer.is_empty() {
            answer.push('\n');
        }
        answer.push_str(detail);
    }
    let answer = chat_ui::ChatMessage {
        id: "inbox-qa-answer".into(),
        role: chat_ui::Role::Assistant,
        state: stage,
        text: answer,
        citations: findings
            .iter()
            .map(|finding| chat_ui::Citation {
                kind: citation_kind(finding.source.kind),
                label: format!("{} · {}", finding.label, finding.observed_at),
            })
            .collect(),
    };
    let sources = turn_id
        .filter(|_| !answer.citations.is_empty())
        .map(|turn_id| answer_sources(turn_id, &answer.citations, qa.sources_open, kit));
    Some(
        rune::col()
            .key("inbox-qa-exchange")
            .w_full()
            .min_w(0.0)
            .gap(8.0)
            .child(chat_ui::bubble(&question, kit))
            .child(
                chat_ui::bubble_with_sources(&answer, sources, kit)
                    .key("inbox-qa-live-answer")
                    .live_region(LiveRegion::Polite),
            ),
    )
}

pub(crate) fn footer(state: &State, kit: &Kit, preferences: ViewPreferences) -> El {
    if let Some(problem) = embedded_problem(state) {
        return footer_layer(problem_content(state, problem, kit), preferences);
    }
    if !state.inbox_qa_visible() {
        return nudge(state, kit, preferences);
    }
    let qa = &state.inbox_qa;
    let phase = state.inbox_qa_settings().map(|settings| settings.phase);
    if phase != Some(QaPhase::Ready) {
        let label = if phase == Some(QaPhase::Starting) {
            "Preparing backend inference…"
        } else {
            unavailable_nudge(state)
        };
        return footer_layer(hint(label, kit).role("status"), preferences);
    }
    if !state.permission(Permission::SubmitInteractive) {
        return footer_layer(hint("This registration can view answers. Sending a question requires interactive access.", kit), preferences);
    }
    let input = rune_kit::TextArea::new(&qa.draft)
        .label("Ask about workstation health")
        .placeholder("Ask about workstation health")
        .auto_grow(2, 6)
        .wrap_anywhere()
        .disabled(qa.pending.is_some())
        .on_edit(|state: &mut State| &mut state.inbox_qa.draft)
        .on_key(|state: &mut State, event: &KeyEvent, tx| {
            if event.key != Key::Enter {
                return false;
            }
            if assistant::enter_action(event.mods.shift, state.inbox_qa.draft.is_composing())
                != assistant::EnterAction::Send
            {
                return false;
            }
            if !tx.key_repeat() {
                state.send_inbox_question(tx);
            }
            true
        })
        .build(kit)
        .key(COMPOSER)
        .w_full();
    let mut row = rune::row().w_full().flex_wrap().center_y().gap(7.0);
    if let Some(source) = &qa.source {
        row = row.child(
            ActionButton::new(format!("Remove {} source", source.kind.to_lowercase()))
                .key("inbox-qa-clear-source")
                .finish(MetalFinish::Gunmetal)
                .disabled(qa.pending.is_some())
                .on_press(|state: &mut State| state.inbox_qa.source = None)
                .build(kit, preferences, None),
        );
    } else {
        let current = assistant::current_context(state);
        if optional_source(&current).is_some() {
            row = row.child(
                ActionButton::new(format!("Include {}", current.kind.to_lowercase()))
                    .key("inbox-qa-pin-source")
                    .finish(MetalFinish::Gunmetal)
                    .disabled(qa.pending.is_some())
                    .on_press(move |state: &mut State| {
                        state.inbox_qa.source = Some(current.clone())
                    })
                    .build(kit, preferences, None),
            );
        }
    }
    row = row.child(rune::spacer());
    if qa.active().is_some() {
        row = row.child(
            ActionButton::new("Stop")
                .key("inbox-qa-stop")
                .finish(MetalFinish::Gunmetal)
                .disabled(qa.mutation_in_flight())
                .on_press(State::stop_inbox_question)
                .build(kit, preferences, None),
        );
    } else {
        row = row.child(
            ActionButton::new("Send")
                .key("inbox-qa-send")
                .finish(MetalFinish::Fractured)
                .disabled(!state.can_send_inbox_qa() || qa.draft.text.trim().is_empty())
                .on_press_tx(State::send_inbox_question)
                .build(kit, preferences, None),
        );
    }
    let mut content = rune::col().w_full().gap(7.0);
    if let Some(status) = qa.status {
        content = content.child(hint(status, kit).role("status"));
    }
    footer_layer(content.child(input).child(row), preferences)
}

fn nudge(state: &State, kit: &Kit, preferences: ViewPreferences) -> El {
    let text = if !state.connected() {
        "Reconnect the selected backend to check inference."
    } else if !state.supports_inbox_qa() {
        "Backend inference is unavailable for this selection."
    } else if state.inbox_qa.change_to == Some(false)
        || state
            .inbox_qa_settings()
            .is_some_and(|settings| settings.phase == QaPhase::Stopping)
    {
        stopping_label(state)
    } else if state.inbox_qa_settings().is_none() {
        state
            .inbox_qa
            .status
            .unwrap_or("Checking backend inference…")
    } else if state
        .inbox_qa_settings()
        .is_some_and(|settings| settings.enabled)
    {
        "This registration does not have access to Inbox answers."
    } else if state.can_manage_inbox_qa() {
        "Enable backend inference in Connection settings to ask questions."
    } else {
        operator_nudge(state)
    };
    let mut content = rune::col()
        .w_full()
        .gap(7.0)
        .child(hint(text, kit).role("status"));
    if !state.connected() || state.can_manage_inbox_qa() {
        content = content.child(
            ActionButton::new("Connection settings")
                .key("inbox-qa-settings")
                .finish(MetalFinish::Gunmetal)
                .on_press_tx(|state: &mut State, tx| {
                    state.close_inbox(tx);
                    state.open_connection_settings();
                    state.inbox_qa.next_settings_at = 0.0;
                    tx.focus("settings-section-title", true);
                })
                .build(kit, preferences, None),
        );
    }
    footer_layer(content, preferences)
}

fn footer_layer(content: El, preferences: ViewPreferences) -> El {
    material::glass(
        rune::col()
            .key("inbox-qa-footer")
            .w_full()
            .min_w(0.0)
            .padding(10.0)
            .child(content),
        GlassSurface::Composer,
        preferences,
    )
}

#[cfg(test)]
mod tests {
    use fleetworks_contracts::chat::QaFinding;
    use rune::Env;
    use rune::Ime;
    use rune::Mods;
    use rune::Runtime;

    use super::*;

    const FINDING: &str = "Actor search is ready.";

    fn turn(state: QaTurnState, cursor: u64) -> QaTurn {
        let mut turn = QaTurn::queued(
            "fixture-turn".into(),
            "fixture-request".into(),
            "Can actor search receive queries?".into(),
        );
        turn.state = state;
        turn.cursor = cursor;
        if cursor >= 2 {
            turn.findings.push(QaFinding::new(
                ChatSourceReference::new(ChatSourceKind::Readiness, "actor-search".into()).unwrap(),
                "Actor search".into(),
                FINDING.into(),
                0,
                FINDING.len() as u32,
                "2026-01-01T00:00:00Z".into(),
            ));
        }
        turn
    }

    fn active_drawer(size: (f32, f32)) -> Runtime<State> {
        let mut state = crate::tests::registered_fixture();
        state.page = crate::app::Page::Workstation;
        state
            .connection
            .capabilities
            .as_mut()
            .unwrap()
            .permissions
            .push(Permission::SubmitInteractive);
        state.accessibility.fixture_override = Some(crate::accessibility::FixtureAccessibility {
            reduced_motion: true,
            ..Default::default()
        });
        state.seed_inbox_qa_drawer_fixture();
        state.seed_inbox_qa_fixture(
            QaSettings::new(true, QaPhase::Ready, 1, 1, true),
            Some(turn(QaTurnState::Running, 1)),
        );
        let mut runtime = Runtime::new(
            state,
            crate::app::view,
            size,
            Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        runtime.frame(0.0);
        runtime.frame(0.5);
        runtime
    }

    fn accessibility_nodes_with_key(node: &serde_json::Value, key: &str) -> usize {
        usize::from(node["key"] == key)
            + node["children"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|child| accessibility_nodes_with_key(child, key))
                .sum::<usize>()
    }

    #[test]
    fn qa_drawer_pins_composer_and_updates_one_answer_at_both_window_sizes() {
        for size in [(640.0, 480.0), (1280.0, 800.0)] {
            let mut runtime = active_drawer(size);
            let (drawer, _) = runtime.presented("inbox-body").unwrap();
            let (scroll, _) = runtime.presented("inbox-scroll").unwrap();
            let (footer, _) = runtime.presented("inbox-qa-footer").unwrap();
            assert!((drawer[0] + drawer[2] - size.0).abs() < 0.5);
            assert!((drawer[1] + drawer[3] - size.1).abs() < 0.5);
            assert!(scroll[1] + scroll[3] <= footer[1] + 0.5);
            assert!((footer[1] + footer[3] - size.1).abs() < 1.5);
            let (exchange, _) = runtime.presented("inbox-qa-exchange").unwrap();
            assert!((scroll[1] + scroll[3] - exchange[1] - exchange[3] - 12.0).abs() < 0.5);
            assert!(runtime.find("inbox-qa-stop").is_some());
            assert!(runtime.find(SOURCES_TOGGLE).is_none());
            let unread = runtime.state.inbox.unread();

            runtime.update(|state, _| {
                state.inbox_qa.adopt(Some(turn(QaTurnState::Running, 2)));
            });
            runtime.frame(1.0);
            assert!(runtime.find("chat-citation-0").is_none());
            let elements = runtime.elements();
            let toggle = elements
                .iter()
                .find(|el| el.key.as_deref() == Some(SOURCES_TOGGLE))
                .expect("Sources disclosure for the cited finding");
            assert_eq!(toggle.role, "button");
            assert_eq!(toggle.label.as_deref(), Some("Sources (1)"));
            assert_eq!(toggle.expanded, Some(false));
            assert!(runtime.accessibility_json().to_string().contains(FINDING));
            let (exchange, _) = runtime.presented("inbox-qa-exchange").unwrap();
            assert!((scroll[1] + scroll[3] - exchange[1] - exchange[3] - 12.0).abs() < 0.5);

            press_disclosure(&mut runtime, SOURCES_TOGGLE, 1.25);
            let elements = runtime.elements();
            let toggle = elements
                .iter()
                .find(|el| el.key.as_deref() == Some(SOURCES_TOGGLE))
                .unwrap();
            assert_eq!(toggle.expanded, Some(true));
            assert!(toggle.focused);
            let citation = elements
                .iter()
                .find(|el| el.key.as_deref() == Some("chat-citation-0"))
                .expect("expanded citation");
            assert_eq!(
                citation.text.as_deref(),
                Some("Readiness: Actor search · 2026-01-01T00:00:00Z")
            );
            assert!(citation.rect[1] >= scroll[1] - 0.5);
            assert!(citation.rect[1] + citation.rect[3] <= scroll[1] + scroll[3] + 0.5);

            runtime.update(|state, _| {
                state.inbox_qa.adopt(Some(turn(QaTurnState::Canceled, 3)));
            });
            runtime.frame(1.75);
            assert!(runtime.find("inbox-qa-stop").is_none());
            assert!(runtime.find("chat-citation-0").is_some());
            let tree = runtime.accessibility_json();
            assert_eq!(accessibility_nodes_with_key(&tree, REPLY), 1);
            let rendered = tree.to_string();
            assert_eq!(rendered.matches("Assistant message").count(), 1);
            assert!(rendered.contains("live-polite"));
            assert!(rendered.contains(FINDING) && rendered.contains("Stopped"));
            assert_eq!(runtime.state.inbox.unread(), unread);

            press_disclosure(&mut runtime, SOURCES_TOGGLE, 2.0);
            assert!(runtime.find("chat-citation-0").is_none());
            press_disclosure(&mut runtime, SOURCES_TOGGLE, 2.5);
            assert!(runtime.find("chat-citation-0").is_some());
            runtime.update(|state, _| {
                let mut next = turn(QaTurnState::Answered, 2);
                next.turn_id = "fixture-next-turn".into();
                next.request_id = "fixture-next-request".into();
                state.inbox_qa.adopt(Some(next));
            });
            runtime.frame(3.0);
            assert!(runtime.find(SOURCES_TOGGLE).is_some());
            assert!(runtime.find("chat-citation-0").is_none());

            runtime.update(|state, _| {
                state
                    .inbox_qa
                    .apply_settings(QaSettings::new(false, QaPhase::Off, 2, 2, false), 3.25);
            });
            runtime.frame(3.25);
            assert!(runtime.find(COMPOSER).is_none());
            assert!(runtime.find(REPLY).is_none());
            assert!(runtime
                .accessibility_json()
                .to_string()
                .contains(operator_nudge(&runtime.state)));
            assert_eq!(runtime.state.inbox.unread(), unread);
        }
    }

    #[test]
    fn informational_notices_keep_the_answer_and_composer_available() {
        let mut runtime = active_drawer((640.0, 480.0));
        for (index, (notice, copy)) in [
            (
                QaTurnNotice::Greeting,
                "Try asking if actor search is ready",
            ),
            (
                QaTurnNotice::OutOfScope,
                "health, readiness, resource usage, and jobs",
            ),
            (
                QaTurnNotice::ObservationUnavailable,
                "relevant current measurement",
            ),
        ]
        .into_iter()
        .enumerate()
        {
            runtime.update(move |state, _| {
                let mut reply = turn(QaTurnState::InsufficientContext, 1);
                reply.turn_id = format!("fixture-notice-{index}");
                reply.notice = Some(notice);
                state.inbox_qa.adopt(Some(reply));
            });
            runtime.frame(1.0 + index as f64 / 4.0);
            let tree = runtime.accessibility_json().to_string();
            assert!(tree.contains(copy));
            assert!(!tree.contains("could not verify an answer"));
            assert!(runtime.find(REPLY).is_some());
            assert!(runtime.find(COMPOSER).is_some());
            assert!(runtime.find(SOURCES_TOGGLE).is_none());
        }
    }

    #[test]
    fn qa_updates_preserve_earlier_scroll_position_and_draft() {
        let mut runtime = active_drawer((640.0, 480.0));
        let mut running = turn(QaTurnState::Running, 2);
        running.findings[0].text = format!("{FINDING}\n").repeat(24);
        let mut completed = running.clone();
        completed.cursor = 3;
        completed.state = QaTurnState::Canceled;
        runtime.update(move |state, _| {
            state.inbox_qa.adopt(Some(running));
            state.inbox_qa.draft = TextEdit::new("An unsent follow-up");
        });
        runtime.frame(0.75);
        assert_eq!(runtime.scroll_at_bottom(crate::inbox::SCROLL), Some(true));

        runtime.update(|_, tx| tx.scroll_to_y(crate::inbox::SCROLL, 0.0));
        runtime.frame(1.0);
        let (older, _) = runtime.presented("inbox-group-connection-toggle").unwrap();
        assert_eq!(runtime.scroll_at_bottom(crate::inbox::SCROLL), Some(false));

        runtime.update(move |state, _| state.inbox_qa.adopt(Some(completed)));
        runtime.frame(1.25);
        assert_eq!(
            runtime
                .presented("inbox-group-connection-toggle")
                .unwrap()
                .0,
            older
        );
        assert_eq!(runtime.scroll_at_bottom(crate::inbox::SCROLL), Some(false));
        assert_eq!(runtime.state.inbox_qa.draft.text, "An unsent follow-up");
    }

    fn press_disclosure(runtime: &mut Runtime<State>, key: &'static str, now: f64) {
        runtime.update(|_, tx| {
            tx.scroll_into_view(key);
            tx.focus(key, true);
        });
        runtime.frame(now);
        assert!(runtime
            .elements()
            .iter()
            .any(|element| { element.key.as_deref() == Some(key) && element.focused }));
        runtime.key(KeyEvent {
            key: Key::Text(" ".into()),
            mods: Mods::default(),
        });
        runtime.frame(now + 0.25);
    }

    #[test]
    fn embedded_inference_failures_keep_notifications_available_and_distinguish_recovery() {
        let cases = [
            (
                QaAvailabilityReason::PackMissing,
                "Embedded QA pack is missing",
                "Install the optional Inbox QA pack",
            ),
            (
                QaAvailabilityReason::PackInvalid,
                "Embedded QA pack could not be verified",
                "Reinstall the Inbox QA pack",
            ),
            (
                QaAvailabilityReason::SandboxUnsupported,
                "Embedded inference isn’t supported on this host",
                "requires Landlock ABI 3 or newer",
            ),
            (
                QaAvailabilityReason::WorkerStartFailed,
                "Embedded inference worker could not start",
                "Check the Embedded companion logs",
            ),
        ];
        for size in [(640.0, 480.0), (1280.0, 800.0)] {
            let mut runtime = active_drawer(size);
            let unread = runtime.state.inbox.unread();
            for (index, (reason, status, details)) in cases.into_iter().enumerate() {
                let now = 1.0 + index as f64;
                runtime.update(|state, _| {
                    state.backend_kind = BackendKind::Local;
                    let mut settings = QaSettings::new(true, QaPhase::Unavailable, 2, 2, true);
                    settings.availability_reason = Some(reason);
                    state.inbox_qa.apply_settings(settings, now);
                });
                runtime.frame(now);
                let tree = runtime.accessibility_json().to_string();
                assert!(tree.contains(status), "{size:?} {reason:?}");
                assert!(!tree.contains(details));
                assert!(runtime.find(COMPOSER).is_none());
                assert!(runtime.find("inbox-filter").is_some());
                assert!(runtime.find("inbox-group-connection-toggle").is_some());
                assert_eq!(runtime.state.inbox.unread(), unread);

                press_disclosure(&mut runtime, DETAILS_TOGGLE, now + 0.25);
                assert!(runtime.accessibility_json().to_string().contains(details));
                let (footer, _) = runtime.presented("inbox-qa-footer").unwrap();
                let (scroll, _) = runtime.presented("inbox-scroll").unwrap();
                assert!(scroll[1] + scroll[3] <= footer[1] + 0.5);
                assert!((footer[1] + footer[3] - size.1).abs() < 1.5);
            }
        }
    }

    #[test]
    fn connection_settings_discloses_the_embedded_host_prerequisite() {
        for size in [(640.0, 480.0), (1280.0, 800.0)] {
            let mut runtime = active_drawer(size);
            runtime.update(|state, _| {
                state.backend_kind = BackendKind::Local;
                let mut settings = QaSettings::new(true, QaPhase::Unavailable, 2, 2, true);
                settings.availability_reason = Some(QaAvailabilityReason::SandboxUnsupported);
                state.inbox_qa.apply_settings(settings, 1.0);
                state.inbox.close();
                state.open_connection_settings();
            });
            runtime.frame(1.0);
            runtime.frame(1.25);
            let tree = runtime.accessibility_json().to_string();
            assert!(tree.contains("Embedded inference isn’t supported on this host"));
            assert!(tree.contains("Turn Off"));
            assert!(!tree.contains("Landlock"));

            press_disclosure(&mut runtime, DETAILS_TOGGLE, 1.5);
            let tree = runtime.accessibility_json();
            assert_eq!(accessibility_nodes_with_key(&tree, DETAILS), 1);
            assert!(tree
                .to_string()
                .contains("requires Landlock ABI 3 or newer"));
        }
    }

    #[test]
    fn qa_composer_preserves_shift_enter_and_ime_commit() {
        let mut runtime = active_drawer((640.0, 480.0));
        runtime.update(|_, tx| tx.focus(COMPOSER, true));
        runtime.frame(0.75);
        assert!(runtime.state.inbox_qa.draft.focused);
        runtime.key(KeyEvent {
            key: Key::Text("Health".into()),
            mods: Mods::default(),
        });
        runtime.frame(0.875);
        assert_eq!(runtime.state.inbox_qa.draft.text, "Health");
        runtime.key(KeyEvent {
            key: Key::Enter,
            mods: Mods {
                shift: true,
                ..Mods::default()
            },
        });
        runtime.frame(1.0);
        assert_eq!(runtime.state.inbox_qa.draft.text, "Health\n");

        runtime.ime(Ime::Preedit("確認".into(), None));
        runtime.frame(1.25);
        assert!(runtime.state.inbox_qa.draft.is_composing());
        runtime.key(KeyEvent {
            key: Key::Enter,
            mods: Mods::default(),
        });
        runtime.frame(1.5);
        assert_eq!(runtime.state.inbox_qa.draft.text, "Health\n");
        assert!(runtime.state.inbox_qa.pending.is_none());

        runtime.ime(Ime::Commit("確認".into()));
        runtime.frame(1.75);
        assert_eq!(runtime.state.inbox_qa.draft.text, "Health\n確認");
        let (two_lines, _) = runtime.presented(COMPOSER).unwrap();
        for rows in [6, 10] {
            runtime
                .update(|state, _| state.inbox_qa.draft = TextEdit::new("detail\n".repeat(rows)));
            runtime.frame(2.0 + rows as f64);
            runtime.frame(2.5 + rows as f64);
            let (height, _) = runtime.presented(COMPOSER).unwrap();
            assert!(height[3] > two_lines[3]);
            if rows == 10 {
                assert!(height[3] <= two_lines[3] + 75.0);
                assert!((height[1] + height[3]) < 480.0);
            }
        }
    }

    #[test]
    fn definitive_rejection_restores_the_draft_and_previous_exchange() {
        let mut runtime = active_drawer((640.0, 480.0));
        let draft = "Is actor seeding complete?";
        runtime.update(|state, tx| {
            state.fixture = false;
            assert!(state.sync_inbox_qa_binding());
            state.fixture = true;
            let context = MutationContext::new(state.registration.context.as_ref().unwrap());
            state.inbox_qa.adopt(Some(turn(QaTurnState::Answered, 3)));
            state.inbox_qa.draft = TextEdit::new(draft);
            state.inbox_qa.pending = Some(Pending {
                request: QaTurnRequest::new("new-request".into(), context, draft.into(), None)
                    .unwrap(),
            });
            tx.scroll_to_bottom_instant("inbox-scroll");
        });
        runtime.frame(1.0);
        assert!(!runtime.accessibility_json().to_string().contains(FINDING));

        runtime.update(|state, tx| {
            let ticket = Ticket {
                epoch: state.connection.epoch,
                origin_revision: state.origin_revision,
                request_revision: state.inbox_qa.request_revision,
            };
            state.receive_inbox_qa(ticket, Reply::Submit(Err(ClientError::Status(429))), tx);
        });
        runtime.frame(1.5);
        assert_eq!(runtime.state.inbox_qa.draft.text, draft);
        assert!(runtime.state.inbox_qa.draft.focused);
        assert!(runtime.state.inbox_qa.pending.is_none());
        assert!(runtime.accessibility_json().to_string().contains(FINDING));
        assert!(runtime.find("inbox-qa-send").is_some());
    }

    #[test]
    fn target_switch_retires_the_draft_and_stale_replay_cannot_replace_a_turn() {
        let mut state = crate::tests::registered_fixture();
        state.seed_inbox_qa_fixture(
            QaSettings::new(true, QaPhase::Ready, 1, 1, true),
            Some(turn(QaTurnState::Answered, 3)),
        );
        state.inbox_qa.adopt(Some(turn(QaTurnState::Running, 1)));
        assert_eq!(
            state.inbox_qa.latest.as_ref().unwrap().state,
            QaTurnState::Answered
        );
        state.fixture = false;
        assert!(state.sync_inbox_qa_binding());
        state.inbox_qa.draft = TextEdit::new("Discard this draft on target switch");
        state.inbox_qa.detach();
        assert!(!state.inbox_qa.draft.text.is_empty());

        state.connection.origin = TextEdit::new("https://other.example.test");
        assert!(state.sync_inbox_qa_binding());
        assert!(state.inbox_qa.draft.text.is_empty());
        assert!(state.inbox_qa.latest.is_none());
        assert!(!state.inbox_qa_visible());
    }
}
