//! Runtime-neutral Inbox QA state and views. The selected registered backend owns admission,
//! model execution, evidence, and the authoritative latest exchange.

use std::collections::HashSet;
use std::time::Duration;

use fleetworks_contracts::chat::ChatSourceKind;
use fleetworks_contracts::chat::ChatSourceReference;
use fleetworks_contracts::chat::QaAvailabilityReason;
use fleetworks_contracts::chat::QaCancelRequest;
use fleetworks_contracts::chat::QaConversationReply;
use fleetworks_contracts::chat::QaDecisionAvailabilityReason;
use fleetworks_contracts::chat::QaDecisionPreference;
use fleetworks_contracts::chat::QaDecisionSettings;
use fleetworks_contracts::chat::QaEventPage;
use fleetworks_contracts::chat::QaEventQuery;
use fleetworks_contracts::chat::QaGeneratorPreference;
use fleetworks_contracts::chat::QaGeneratorSettings;
use fleetworks_contracts::chat::QaModelTarget;
use fleetworks_contracts::chat::QaPhase;
use fleetworks_contracts::chat::QaRelatedActivity;
use fleetworks_contracts::chat::QaSettings;
use fleetworks_contracts::chat::QaSettingsUpdate;
use fleetworks_contracts::chat::QaStartupStage;
use fleetworks_contracts::chat::QaTurn;
use fleetworks_contracts::chat::QaTurnNotice;
use fleetworks_contracts::chat::QaTurnRequest;
use fleetworks_contracts::chat::QaTurnState;
use fleetworks_contracts::chat::MAX_EVENT_WAIT_SECS;
use fleetworks_contracts::chat::MAX_INPUT_BYTES;
use fleetworks_contracts::chat::QA_HISTORY_FEATURE;
use fleetworks_contracts::chat::QA_MAX_RETAINED_TURNS;
use fleetworks_contracts::chat::QA_MODEL_SETTINGS_FEATURE;
use fleetworks_contracts::chat::QA_TRIAGE_FEATURE;
use fleetworks_contracts::context::MutationContext;
use fleetworks_contracts::context::Permission;
use fleetworks_contracts::context::RegisteredContext;
use fleetworks_contracts::qa_worker::QaContextUsage;
use fleetworks_contracts::Client;
use fleetworks_contracts::ClientError;
use fleetworks_workstation::native::BackendKind;
use rune::El;
use rune::Key;
use rune::KeyEvent;
use rune::LiveRegion;
use rune::TextEdit;
use rune::Tx;
use rune_kit::ControlSize;
use rune_kit::IconButton;
use rune_kit::IconButtonVariant;
use rune_kit::Kit;
use rune_kit::ProgressCircle;
use rune_kit::RadioItem;

use crate::accessibility::ViewPreferences;
use crate::activity_triage::Anchor;
use crate::assistant;
use crate::chat_ui;
use crate::connection::GatewayEffects;
use crate::connection::RequestTask;
use crate::icons;
use crate::material::{self, GlassSurface, MetalFinish};
use crate::theme;
use crate::theme::StatusTone;
use crate::ui_components::ActionButton;
use crate::workstation_settings::SettingsSection;
use crate::State;

const SETTINGS_REFRESH: f64 = 5.0;
const HISTORY_REFRESH: f64 = 15.0;
const STARTUP_REFRESH: f64 = 1.0;
const MIN_EVENT_INTERVAL: f64 = 0.2;
const GREETING: &str = "inbox-qa-greeting";
const COMPOSER: &str = "inbox-qa-composer";
const SOURCES_TOGGLE: &str = "inbox-qa-sources-toggle";
const SOURCES_CONTENT: &str = "inbox-qa-sources";
const HISTORY_PREVIOUS: &str = "inbox-qa-history-previous";
const HISTORY_NEXT: &str = "inbox-qa-history-next";
const CONTEXT_DIAL: &str = "inbox-qa-context";
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
    History,
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
    History,
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
            Self::History => Kind::History,
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
    History(Result<Vec<QaTurn>, ClientError>),
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ModelRole {
    Generator,
    Decision,
}

#[derive(Clone, Copy)]
enum ModelPreference {
    Generator(QaGeneratorPreference),
    Decision(QaDecisionPreference),
}

impl ModelPreference {
    fn role(self) -> ModelRole {
        match self {
            Self::Generator(_) => ModelRole::Generator,
            Self::Decision(_) => ModelRole::Decision,
        }
    }
}

#[derive(Default)]
struct TranscriptHistory {
    turns: Vec<QaTurn>,
    selected: Option<String>,
    sources_open: bool,
    next_refresh_at: f64,
}

impl TranscriptHistory {
    fn select(&mut self, turn: Option<String>) {
        self.selected = turn;
        self.sources_open = false;
    }
}

#[derive(Clone, Copy)]
enum HistoryDirection {
    Previous,
    Next,
}

impl HistoryDirection {
    fn key(self) -> &'static str {
        match self {
            Self::Previous => HISTORY_PREVIOUS,
            Self::Next => HISTORY_NEXT,
        }
    }
}

pub(crate) struct InboxQa {
    binding: Option<Binding>,
    settings: Option<QaSettings>,
    latest: Option<QaTurn>,
    history: TranscriptHistory,
    pending: Option<Pending>,
    draft: TextEdit,
    source: Option<ChatSourceReference>,
    status: Option<&'static str>,
    settings_loaded: bool,
    details_open: bool,
    sources_open: bool,
    startup_seen: bool,
    pending_inference: Option<bool>,
    pending_model: Option<ModelRole>,
    model_save_error: Option<(ModelRole, &'static str)>,
    settings_refresh_required: bool,
    #[cfg(test)]
    captured_update: Option<QaSettingsUpdate>,
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
            history: TranscriptHistory::default(),
            pending: None,
            draft: TextEdit::new(""),
            source: None,
            status: None,
            settings_loaded: false,
            details_open: false,
            sources_open: false,
            startup_seen: false,
            pending_inference: None,
            pending_model: None,
            model_save_error: None,
            settings_refresh_required: false,
            #[cfg(test)]
            captured_update: None,
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
        self.history = TranscriptHistory::default();
        self.request_resume();
    }

    fn retire(&mut self) {
        self.latest = None;
        self.history = TranscriptHistory::default();
        self.sources_open = false;
        self.pending = None;
        self.draft = TextEdit::new("");
        self.source = None;
    }

    pub(crate) fn request_resume(&mut self) {
        self.resume = true;
        self.next_turn_at = 0.0;
        self.next_settings_at = 0.0;
        self.history.next_refresh_at = 0.0;
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

    fn settings_change_pending(&self) -> bool {
        self.pending_inference.is_some()
            || self.pending_model.is_some()
            || self.settings_refresh_required
            || self
                .request
                .as_ref()
                .is_some_and(|task| task.kind == Kind::Update)
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
            self.history.next_refresh_at = 0.0;
        } else if self
            .latest
            .as_ref()
            .is_some_and(|turn| !turn.state.is_terminal())
            && turn.as_ref().is_some_and(|turn| turn.state.is_terminal())
        {
            self.history.next_refresh_at = 0.0;
        }
        self.latest = turn;
        self.reconcile_history_selection();
    }

    fn primary_turn(&self) -> Option<&QaTurn> {
        self.latest.as_ref().or_else(|| self.history.turns.first())
    }

    fn earlier_turns(&self) -> impl Iterator<Item = &QaTurn> {
        let primary = if self.pending.is_none() {
            self.primary_turn()
        } else {
            None
        };
        self.history
            .turns
            .iter()
            .filter(move |turn| primary.is_none_or(|current| turn.turn_id != current.turn_id))
    }

    fn history_position(&self) -> usize {
        self.history.selected.as_ref().map_or(0, |id| {
            self.earlier_turns()
                .position(|turn| &turn.turn_id == id)
                .map_or(0, |index| index + 1)
        })
    }

    fn browsed_turn(&self) -> Option<&QaTurn> {
        let id = self.history.selected.as_ref()?;
        self.earlier_turns().find(|turn| &turn.turn_id == id)
    }

    fn reconcile_history_selection(&mut self) {
        if self.history.selected.is_some() && self.browsed_turn().is_none() {
            self.history.select(None);
        }
    }

    fn replace_history(&mut self, turns: Vec<QaTurn>, now: f64) {
        let mut seen = HashSet::new();
        self.history.turns = turns
            .into_iter()
            .take(QA_MAX_RETAINED_TURNS)
            .filter(|turn| {
                turn.state.is_terminal()
                    && turn.provisional_reply.is_none()
                    && seen.insert(turn.turn_id.clone())
            })
            .collect();
        self.history.next_refresh_at = now + HISTORY_REFRESH;
        self.reconcile_history_selection();
    }

    fn can_browse(&self, direction: HistoryDirection) -> bool {
        let position = self.history_position();
        match direction {
            HistoryDirection::Previous => self.earlier_turns().nth(position).is_some(),
            HistoryDirection::Next => position != 0,
        }
    }

    fn browse(&mut self, direction: HistoryDirection) -> bool {
        let position = self.history_position();
        let next = match direction {
            HistoryDirection::Previous => {
                let Some(turn) = self.earlier_turns().nth(position) else {
                    return false;
                };
                Some(turn.turn_id.clone())
            }
            HistoryDirection::Next if position != 0 => position
                .checked_sub(2)
                .and_then(|index| self.earlier_turns().nth(index))
                .map(|turn| turn.turn_id.clone()),
            HistoryDirection::Next => return false,
        };
        self.history.select(next);
        true
    }

    fn apply_settings(&mut self, settings: QaSettings, now: f64) {
        let generation_changed = self
            .settings
            .as_ref()
            .is_some_and(|old| settings.generation != old.generation);
        let model_settings = settings.generator.is_some() && settings.decision.is_some();
        if !settings.enabled || (generation_changed && !model_settings) {
            self.retire();
            self.resume = true;
        } else if generation_changed {
            // Completed exchanges remain usable across roles. Recover the canonical state of
            // any previously active exchange instead of showing its stale draft.
            self.pending = None;
            if self.active().is_some() {
                self.latest = None;
                self.sources_open = false;
            }
            self.request_resume();
        }
        if self.settings.and_then(|old| old.availability_reason) != settings.availability_reason {
            self.details_open = false;
        }
        self.settings = Some(settings);
        self.settings_loaded = true;
        self.pending_inference = None;
        self.pending_model = None;
        self.settings_refresh_required = false;
        let tracking_startup =
            settings.startup_stage.is_some() || settings.phase == QaPhase::Starting;
        self.next_settings_at = now
            + if tracking_startup {
                STARTUP_REFRESH
            } else {
                SETTINGS_REFRESH
            };
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
        self.history.select(None);
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

    /// Install synthetic retained transcript exchanges for a native fixture.
    #[doc(hidden)]
    pub fn seed_inbox_qa_history_fixture(&mut self, turns: Vec<QaTurn>) {
        if self.fixture {
            if let Some(capabilities) = &mut self.connection.capabilities {
                if !capabilities
                    .features
                    .iter()
                    .any(|feature| feature == QA_HISTORY_FEATURE)
                {
                    capabilities.features.push(QA_HISTORY_FEATURE.into());
                }
            }
            self.inbox_qa.replace_history(turns, self.clock);
        }
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
        self.is_local_backend()
            && self.connected()
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

    fn supports_inbox_qa_history(&self) -> bool {
        self.supports_inbox_qa()
            && self.permission(Permission::ReadWorkstationActivity)
            && self
                .connection
                .capabilities
                .as_ref()
                .is_some_and(|capabilities| {
                    capabilities
                        .features
                        .iter()
                        .any(|feature| feature == QA_HISTORY_FEATURE)
                })
    }

    fn browsed_inbox_qa_turn(&self) -> Option<&QaTurn> {
        if self.supports_inbox_qa_history() {
            self.inbox_qa.browsed_turn()
        } else {
            None
        }
    }

    fn displayed_inbox_qa_turn(&self) -> Option<&QaTurn> {
        let qa = &self.inbox_qa;
        if let Some(turn) = self.browsed_inbox_qa_turn() {
            Some(turn)
        } else if qa.pending.is_some() {
            None
        } else if self.supports_inbox_qa_history() {
            qa.primary_turn()
        } else {
            qa.latest.as_ref()
        }
    }

    pub(crate) fn supports_workstation_triage(&self) -> bool {
        self.supports_inbox_qa()
            && self
                .connection
                .capabilities
                .as_ref()
                .is_some_and(|capabilities| {
                    capabilities
                        .features
                        .iter()
                        .any(|feature| feature == QA_TRIAGE_FEATURE)
                })
    }

    fn supports_model_settings(&self) -> bool {
        self.supports_inbox_qa()
            && self
                .connection
                .capabilities
                .as_ref()
                .is_some_and(|capabilities| {
                    capabilities
                        .features
                        .iter()
                        .any(|feature| feature == QA_MODEL_SETTINGS_FEATURE)
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

    pub(crate) fn assistant_settings_visible(&self) -> bool {
        self.inbox_qa.pending_inference != Some(false)
            && self
                .inbox_qa_settings()
                .is_some_and(|settings| settings.enabled && settings.phase != QaPhase::Stopping)
    }

    pub(crate) fn triage_generation(&self) -> Option<u64> {
        if !self.supports_workstation_triage() || !self.assistant_settings_visible() {
            return None;
        }
        self.inbox_qa_settings().map(|settings| settings.generation)
    }

    pub(crate) fn inbox_qa_visible(&self) -> bool {
        self.assistant_settings_visible()
            && (self.supports_workstation_triage()
                || self.permission(Permission::ReadWorkstationActivity))
    }

    pub(crate) fn can_prefill_inbox_question(&self) -> bool {
        self.supports_workstation_triage()
            && self.inbox_qa_visible()
            && self.permission(Permission::SubmitInteractive)
            && self.inbox_qa.pending.is_none()
            && self.inbox_qa.active().is_none()
    }

    pub(crate) fn prefill_inbox_question(
        &mut self,
        source: &ChatSourceReference,
        anchor: Anchor,
        tx: &mut Tx,
    ) {
        if !self.can_prefill_inbox_question() {
            return;
        }
        let prompt = match source.kind {
            ChatSourceKind::Job => "What happened with this job?",
            ChatSourceKind::Readiness => "What happened with this readiness check?",
            _ => return,
        };
        if !self.inbox.open && !self.has_active_confirmation() {
            debug_assert_eq!(anchor, Anchor::Job);
            self.open_inbox_from("triage-explain");
        }
        if !self.inbox.open {
            return;
        }
        if self.inbox_qa.draft.text.trim().is_empty() {
            self.inbox_qa.draft = TextEdit::new(prompt);
        }
        self.inbox_qa.source = Some(source.clone());
        tx.focus(COMPOSER, true);
    }

    pub(crate) fn canonical_related_activity(&self) -> Option<&QaRelatedActivity> {
        if !self.supports_workstation_triage()
            || !self.inbox_qa_visible()
            || self.browsed_inbox_qa_turn().is_some()
        {
            return None;
        }
        let qa = &self.inbox_qa;
        if qa.pending.is_some() {
            return None;
        }
        qa.latest
            .as_ref()
            .filter(|turn| {
                matches!(
                    turn.state,
                    QaTurnState::Answered | QaTurnState::InsufficientContext
                )
            })
            .and_then(|turn| turn.related_activity.as_ref())
    }

    fn can_send_inbox_qa(&self) -> bool {
        self.inbox_qa_visible()
            && self.permission(Permission::SubmitInteractive)
            && self.inbox_qa_settings().is_some_and(|settings| {
                settings.phase == QaPhase::Ready || self.supports_workstation_triage()
            })
            && !self.inbox_qa.resume
            && self.inbox_qa.pending.is_none()
            && self.inbox_qa.active().is_none()
            && !self.inbox_qa.mutation_in_flight()
            && !self.inbox_qa.settings_change_pending()
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
        let mut changed = self.sync_inbox_qa_binding();
        let supports_history = self.supports_inbox_qa_history();
        if !supports_history && !self.inbox_qa.history.turns.is_empty() {
            self.inbox_qa.history = TranscriptHistory::default();
            changed = true;
        }
        if self.fixture || !self.supports_inbox_qa() || self.inbox_qa.request.is_some() {
            return changed;
        }
        let qa = &self.inbox_qa;
        let watch_settings = !qa.settings_loaded
            || qa.settings_change_pending()
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
        let restore_usage = self.settings_open
            && self.settings_section == SettingsSection::Assistant
            && qa.resume
            && qa
                .settings
                .is_some_and(|settings| settings.phase == QaPhase::Ready);
        if !self.inbox_qa_visible()
            || (!self.inbox.open && !restore_usage && qa.active().is_none() && qa.pending.is_none())
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
        } else if self.inbox.open && supports_history && now >= qa.history.next_refresh_at {
            Some(Request::History)
        } else {
            if self.inbox.open && supports_history {
                tx.wake_after(Duration::from_secs_f64(qa.history.next_refresh_at - now));
            }
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
        if !client.is_local_ipc() {
            return false;
        }
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
        let may_read_history = self.supports_inbox_qa_history();
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
                qa.model_save_error = None;
            }
            Reply::Settings(Err(error)) => {
                qa.status = Some(status_message(error));
                if denied(error) {
                    qa.retire();
                    qa.settings_loaded = false;
                }
                qa.next_settings_at = qa.retry_after(now);
            }
            Reply::Update(Err(error)) => {
                let conflict = matches!(error, ClientError::Status(409));
                let refresh_required = conflict || !admission_rejected(error);
                let message = if conflict {
                    "Settings changed. Review the current preference and choose again."
                } else {
                    status_message(error)
                };
                if let Some(role) = qa.pending_model.take() {
                    qa.model_save_error = Some((role, message));
                } else {
                    qa.status = Some(message);
                }
                if admission_rejected(error) {
                    qa.pending_inference = None;
                }
                qa.settings_refresh_required = refresh_required;
                if denied(error) {
                    qa.retire();
                    qa.settings_loaded = false;
                }
                qa.next_settings_at = if refresh_required {
                    now
                } else {
                    qa.retry_after(now)
                };
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
            Reply::History(Ok(turns)) if may_read_history => qa.replace_history(turns, now),
            Reply::History(Ok(_)) => qa.history = TranscriptHistory::default(),
            Reply::History(Err(error)) if denied(error) => {
                qa.retire();
                qa.settings_loaded = false;
                qa.next_settings_at = now;
            }
            Reply::History(Err(_)) => {
                qa.history = TranscriptHistory {
                    next_refresh_at: now + HISTORY_REFRESH,
                    ..TranscriptHistory::default()
                };
            }
            Reply::Events(Ok(page)) => {
                if qa
                    .latest
                    .as_ref()
                    .is_some_and(|turn| turn.turn_id == page.turn.turn_id)
                {
                    // Each page carries the cumulative draft. Apply it once per page so visual
                    // updates are bounded and a lost event or reconnect can recover the turn.
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
            || self.inbox_qa.settings_change_pending()
            || self.inbox_qa.mutation_in_flight()
        {
            return;
        }
        let (Some(context), Some(settings)) =
            (&self.registration.context, self.inbox_qa_settings())
        else {
            return;
        };
        if settings.phase == QaPhase::Stopping || enabled == settings.enabled {
            return;
        }
        let update =
            QaSettingsUpdate::new(MutationContext::new(context), settings.revision, enabled);
        if self.dispatch_inbox_qa(Request::Update(update)) {
            self.inbox_qa.pending_inference = Some(enabled);
            self.inbox_qa.status = None;
        }
    }

    fn set_model_preference(&mut self, preference: ModelPreference) {
        if !self.supports_model_settings()
            || !self.assistant_settings_visible()
            || !self.can_manage_inbox_qa()
            || self.inbox_qa.settings_change_pending()
            || self.inbox_qa.mutation_in_flight()
        {
            return;
        }
        let (Some(context), Some(settings)) =
            (&self.registration.context, self.inbox_qa_settings())
        else {
            return;
        };
        let update =
            QaSettingsUpdate::preferences(MutationContext::new(context), settings.revision);
        let update = match preference {
            ModelPreference::Generator(preference)
                if settings
                    .generator
                    .is_some_and(|role| role.preference != preference) =>
            {
                update.with_generator_preference(preference)
            }
            ModelPreference::Decision(preference)
                if settings
                    .decision
                    .is_some_and(|role| role.preference != preference) =>
            {
                update.with_decision_preference(preference)
            }
            _ => return,
        };
        let role = preference.role();
        #[cfg(test)]
        if self.fixture {
            self.inbox_qa.request_revision = self.inbox_qa.request_revision.wrapping_add(1);
            self.inbox_qa.captured_update = Some(update);
            self.inbox_qa.pending_model = Some(role);
            self.inbox_qa.model_save_error = None;
            return;
        }
        if self.dispatch_inbox_qa(Request::Update(update)) {
            self.inbox_qa.pending_model = Some(role);
            self.inbox_qa.model_save_error = None;
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
        let source = self.inbox_qa.source.clone();
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
            self.inbox_qa.history.select(None);
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

    fn browse_inbox_qa_history(&mut self, direction: HistoryDirection, tx: &mut Tx) {
        if !self.inbox.open
            || !self.inbox_qa_visible()
            || !self.supports_inbox_qa_history()
            || !self.inbox_qa.browse(direction)
        {
            return;
        }
        if !self.inbox_qa.can_browse(direction) {
            let opposite = match direction {
                HistoryDirection::Previous => HISTORY_NEXT,
                HistoryDirection::Next => HISTORY_PREVIOUS,
            };
            tx.focus(opposite, tx.focus_visible());
        }
        tx.scroll_to_bottom_instant(crate::inbox::SCROLL);
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
                Request::History => Reply::History(client.qa_history().await),
                Request::Events(request) => Reply::Events(client.qa_events(&request).await),
                Request::Cancel(request) => Reply::Cancel(client.cancel_qa_turn(&request).await),
            };
            ui.update(move |state, tx| state.receive_inbox_qa(ticket, reply, tx));
        });
        Some(RequestTask(task.abort_handle()))
    }
}

fn phase_label(state: &State) -> (&'static str, StatusTone) {
    match state.inbox_qa.pending_inference {
        Some(false) => return ("Stopping", StatusTone::Warn),
        Some(true) => return ("Starting", StatusTone::Info),
        None => {}
    }
    if let Some(label) = startup_label(state) {
        return (label, StatusTone::Info);
    }
    match state.inbox_qa_settings() {
        Some(settings) => model_phase(settings.phase),
        None if state.supports_inbox_qa() => ("Checking", StatusTone::Info),
        None => ("Unavailable", StatusTone::Neutral),
    }
}

fn startup_label(state: &State) -> Option<&'static str> {
    if state.inbox_qa.pending_inference.is_some() {
        return None;
    }
    let settings = state.inbox_qa_settings()?;
    if !settings.enabled || matches!(settings.phase, QaPhase::Ready | QaPhase::Stopping) {
        return None;
    }
    match settings.startup_stage? {
        QaStartupStage::DownloadingPack => Some("Downloading QA pack"),
        QaStartupStage::VerifyingPack => Some("Verifying QA pack"),
        QaStartupStage::LoadingModel => Some("Loading QA model"),
        QaStartupStage::Activating => Some("Activating QA model"),
        _ => None,
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
    if !state.is_local_backend()
        || state.inbox_qa.pending_inference.is_some()
        || startup_label(state).is_some()
    {
        return None;
    }
    let settings = state.inbox_qa_settings()?;
    if !settings.enabled || settings.phase != QaPhase::Unavailable {
        return None;
    }
    problem_for_reason(settings.availability_reason?)
}

fn problem_for_reason(reason: QaAvailabilityReason) -> Option<EmbeddedProblem> {
    let problem = match reason {
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
        QaAvailabilityReason::InsufficientResources => EmbeddedProblem {
            status: "Not enough resources for Assistant",
            details: "Free memory or disk space on this workstation, then turn inference off and back on.",
        },
        QaAvailabilityReason::ModelProvisionFailed => EmbeddedProblem {
            status: "Assistant model provisioning failed",
            details: "Check network access and free disk space, then turn inference off and back on.",
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
        .child(hint("Runs on this device", kit));
    let mut actions = rune::row()
        .w_full()
        .flex_wrap()
        .center_y()
        .gap(8.0)
        .children(problem.is_none().then(|| {
            crate::ui_components::status_pill(phase, None, tone, kit)
                .key("inbox-qa-settings-status")
                .live_region(LiveRegion::Polite)
        }));
    if let Some(settings) = state.inbox_qa_settings() {
        if state.can_manage_inbox_qa() {
            let enable = !settings.enabled;
            actions = actions.child(
                ActionButton::new(if enable { "Turn On" } else { "Turn Off" })
                    .key(SETTINGS_CONTROL)
                    .finish(MetalFinish::Gunmetal)
                    .disabled(
                        state.inbox_qa.settings_change_pending()
                            || settings.phase == QaPhase::Stopping,
                    )
                    .on_press(move |state: &mut State| state.set_inbox_inference(enable))
                    .build(kit, preferences, None),
            );
        } else if !settings.enabled && problem.is_none() {
            content = content.child(hint(OPERATOR_NUDGE, kit));
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
        content = content.child(hint(UNAVAILABLE_NUDGE, kit));
    }
    content.child(actions)
}

const OPERATOR_NUDGE: &str = "Ask an authorized workstation manager to enable backend inference.";
const UNAVAILABLE_NUDGE: &str =
    "Embedded inference is unavailable. Check Connection settings for backend status.";

fn model_footprint(target: QaModelTarget) -> String {
    let bytes = target.weight_bytes();
    if bytes >= 1_000_000_000 {
        format!("{:.2} GB of model weights", bytes as f64 / 1_000_000_000.0)
    } else {
        format!("{:.0} MB of model weights", bytes as f64 / 1_000_000.0)
    }
}

const GENERATOR_CHOICES: [(QaGeneratorPreference, &str); 3] = [
    (
        QaGeneratorPreference::Automatic,
        "assistant-generator-automatic",
    ),
    (
        QaGeneratorPreference::FlanT5Small,
        "assistant-generator-small",
    ),
    (
        QaGeneratorPreference::FlanT5Base,
        "assistant-generator-base",
    ),
];

const DECISION_CHOICES: [(QaDecisionPreference, &str); 3] = [
    (QaDecisionPreference::RulesOnly, "assistant-decision-rules"),
    (QaDecisionPreference::Julia1Q8, "assistant-decision-julia"),
    (
        QaDecisionPreference::VllmD3EdgeQ8,
        "assistant-decision-edge",
    ),
];

fn generator_target(preference: QaGeneratorPreference) -> Option<QaModelTarget> {
    match preference {
        QaGeneratorPreference::FlanT5Small => Some(QaModelTarget::FlanT5Small),
        QaGeneratorPreference::FlanT5Base => Some(QaModelTarget::FlanT5Base),
        _ => None,
    }
}

fn generator_label(preference: QaGeneratorPreference) -> &'static str {
    generator_target(preference).map_or("Automatic", QaModelTarget::label)
}

fn decision_label(preference: QaDecisionPreference) -> &'static str {
    match preference {
        QaDecisionPreference::RulesOnly => "Rules only",
        QaDecisionPreference::Julia1Q8 => "Julia-1 Q8",
        QaDecisionPreference::VllmD3EdgeQ8 => "vLLM d3-edge Q8",
        _ => "Unknown",
    }
}

fn decision_reason(reason: QaDecisionAvailabilityReason) -> &'static str {
    match reason {
        QaDecisionAvailabilityReason::AwaitingQualification => {
            "Awaiting qualification on this platform"
        }
        QaDecisionAvailabilityReason::InsufficientResources => {
            "Not enough resources for the preferred decision model"
        }
        QaDecisionAvailabilityReason::PackMissing => "Decision model pack is unavailable",
        QaDecisionAvailabilityReason::WorkerStartFailed => "Decision model could not start",
        _ => "Decision model unavailable",
    }
}

fn model_phase(phase: QaPhase) -> (&'static str, StatusTone) {
    match phase {
        QaPhase::Off => ("Off", StatusTone::Neutral),
        QaPhase::Starting => ("Starting", StatusTone::Info),
        QaPhase::Ready => ("Ready", StatusTone::Ok),
        QaPhase::Stopping => ("Stopping", StatusTone::Warn),
        _ => ("Unavailable", StatusTone::Warn),
    }
}

fn model_preference_status(state: &State, role: ModelRole, kit: &Kit) -> Option<El> {
    let qa = &state.inbox_qa;
    let (message, tone) = if qa.pending_model == Some(role) {
        ("Saving preference…", StatusTone::Info)
    } else {
        (
            qa.model_save_error
                .filter(|(saved_role, _)| *saved_role == role)?
                .1,
            StatusTone::Warn,
        )
    };
    Some(
        hint(message, kit)
            .color(tone.fg())
            .key("assistant-preference-save")
            .role("status")
            .live_region(LiveRegion::Polite),
    )
}

fn role_summary(preferred: &str, effective: &str, reason: Option<&str>, kit: &Kit) -> El {
    rune::col()
        .w_full()
        .gap(4.0)
        .role("status")
        .live_region(LiveRegion::Polite)
        .child(hint(&format!("Preferred · {preferred}"), kit))
        .child(hint(&format!("In use · {effective}"), kit))
        .children(reason.map(|reason| hint(reason, kit)))
}

fn model_selector(
    label: &str,
    key: &str,
    disabled: bool,
    options: impl Iterator<Item = El>,
    kit: &Kit,
) -> El {
    rune::col()
        .key(key.to_owned())
        .role("radiogroup")
        .label(label.to_owned())
        .focus_group()
        .disabled(disabled)
        .gap(8.0)
        .child(
            rune::text(label)
                .font(kit.ty.small.lh(11.0))
                .color(kit.c.text_faint)
                .a11y_hidden(),
        )
        .child(rune::col().key("items").gap(12.0).children(options))
}

fn generator_settings_content(
    state: &State,
    settings: &QaSettings,
    generator: QaGeneratorSettings,
    kit: &Kit,
) -> El {
    let disabled = !state.can_manage_inbox_qa();
    let options = GENERATOR_CHOICES
        .into_iter()
        .enumerate()
        .map(|(index, (preference, key))| {
            let label = generator_label(preference);
            let label = generator_target(preference).map_or_else(
                || label.into(),
                |target| {
                    format!(
                        "{label} · {:.0} MB",
                        target.weight_bytes() as f64 / 1_000_000.0
                    )
                },
            );
            RadioItem::new(label, preference == generator.preference)
                .disabled(disabled)
                .on_select(move |state: &mut State| {
                    state.set_model_preference(ModelPreference::Generator(preference));
                })
                .build(kit)
                .key(key)
                .position_in_set(index + 1, GENERATOR_CHOICES.len())
        });
    let phase = if generator.phase == QaPhase::Ready && generator.effective.is_none() {
        QaPhase::Unavailable
    } else {
        generator.phase
    };
    let (label, tone) = startup_label(state)
        .filter(|_| phase != QaPhase::Ready)
        .map_or_else(|| model_phase(phase), |label| (label, StatusTone::Info));
    let reason = generator
        .reason
        .and_then(problem_for_reason)
        .map(|problem| problem.status)
        .or_else(|| {
            (generator.preference == QaGeneratorPreference::Automatic
                && generator.effective.is_some())
            .then_some("Selected for available workstation resources")
        });
    let usage = state
        .inbox_qa
        .latest
        .as_ref()
        .filter(|turn| {
            state.inbox_qa_visible()
                && generator.effective.is_some()
                && turn.model_target == generator.effective
                && turn.model_generation == Some(settings.generation)
        })
        .and_then(|turn| turn.context);
    rune::col()
        .key("assistant-generator-section")
        .w_full()
        .min_w(0.0)
        .gap(8.0)
        .child(model_selector(
            "Generator",
            "assistant-generator-preference",
            disabled,
            options,
            kit,
        ))
        .child(role_summary(
            generator_label(generator.preference),
            generator.effective.map_or("None", QaModelTarget::label),
            reason,
            kit,
        ))
        .child(
            crate::ui_components::status_pill(label, None, tone, kit)
                .key("assistant-model-status")
                .live_region(LiveRegion::Polite),
        )
        .child(assistant_context_usage(
            QaModelTarget::default().input_tokens(),
            usage,
            kit,
        ))
        .children(model_preference_status(state, ModelRole::Generator, kit))
}

fn decision_settings_content(state: &State, decision: QaDecisionSettings, kit: &Kit) -> El {
    let disabled = !state.can_manage_inbox_qa();
    let options = DECISION_CHOICES
        .into_iter()
        .enumerate()
        .map(|(index, (preference, key))| {
            let label = match preference {
                QaDecisionPreference::Julia1Q8 => "Julia-1 Q8 · 168 MB",
                QaDecisionPreference::VllmD3EdgeQ8 => "vLLM d3-edge Q8 · 806 MB",
                _ => decision_label(preference),
            };
            RadioItem::new(label, preference == decision.preference)
                .disabled(disabled)
                .on_select(move |state: &mut State| {
                    state.set_model_preference(ModelPreference::Decision(preference));
                })
                .build(kit)
                .key(key)
                .position_in_set(index + 1, DECISION_CHOICES.len())
        });
    let phase = match (decision.effective, decision.phase) {
        (QaDecisionPreference::RulesOnly, QaPhase::Off | QaPhase::Ready | QaPhase::Unavailable) => {
            None
        }
        (_, phase) => Some(model_phase(phase)),
    };
    rune::col()
        .key("assistant-decision-section")
        .w_full()
        .min_w(0.0)
        .gap(8.0)
        .child(model_selector(
            "Decision",
            "assistant-decision-preference",
            disabled,
            options,
            kit,
        ))
        .child(role_summary(
            decision_label(decision.preference),
            decision_label(decision.effective),
            decision.reason.map(decision_reason),
            kit,
        ))
        .children(phase.map(|(label, tone)| {
            crate::ui_components::status_pill(label, None, tone, kit)
                .key("assistant-decision-status")
                .live_region(LiveRegion::Polite)
        }))
        .children(model_preference_status(state, ModelRole::Decision, kit))
}

fn assistant_context_usage(capacity: u16, usage: Option<QaContextUsage>, kit: &Kit) -> El {
    rune::col()
        .gap(4.0)
        .child(hint("Context window", kit))
        .child(
            match usage.filter(|usage| usage.window_tokens == capacity) {
                Some(usage) => context_dial(Some(usage), kit).key("assistant-context-usage"),
                None => rune::text(format!("{capacity} tokens · No measured usage"))
                    .font(kit.ty.small_medium)
                    .color(kit.c.std_text)
                    .w_full()
                    .wrap(),
            },
        )
}

pub(crate) fn assistant_settings_content(state: &State, kit: &Kit) -> El {
    let Some(settings) = state.inbox_qa_settings() else {
        return rune::el();
    };
    if state.supports_model_settings() {
        if let (Some(generator), Some(decision)) = (settings.generator, settings.decision) {
            return rune::col()
                .w_full()
                .min_w(0.0)
                .padding(12.0)
                .gap(12.0)
                .children((!state.can_manage_inbox_qa()).then(|| {
                    hint(
                        "Only a workstation manager can change model preferences.",
                        kit,
                    )
                }))
                .child(generator_settings_content(state, settings, generator, kit))
                .child(rune::rule(1.0, theme::hex(theme::BORDER_SUBTLE)).a11y_hidden())
                .child(decision_settings_content(state, decision, kit));
        }
    }
    let target = settings.model_target;
    let (phase, tone) = phase_label(state);
    let status = crate::ui_components::status_pill(phase, None, tone, kit)
        .key("assistant-model-status")
        .live_region(LiveRegion::Polite);
    let model_label = if settings.phase == QaPhase::Ready {
        "Active model"
    } else {
        "Selected model"
    };
    let usage = state
        .inbox_qa
        .latest
        .as_ref()
        .filter(|_| state.inbox_qa_visible())
        .and_then(|turn| turn.context);
    let context = assistant_context_usage(target.input_tokens(), usage, kit);
    let problem = embedded_problem(state);
    let diagnostic = problem
        .as_ref()
        .map(|problem| problem.details)
        .or(state.inbox_qa.status)
        .or_else(|| (phase == "Unavailable").then_some(UNAVAILABLE_NUDGE));
    let model_selected =
        settings.availability_reason != Some(QaAvailabilityReason::InsufficientResources);
    rune::col()
        .w_full()
        .min_w(0.0)
        .padding(12.0)
        .gap(12.0)
        .child(
            rune::row()
                .w_full()
                .min_w(0.0)
                .flex_wrap()
                .gap(8.0)
                .center_y()
                .children(model_selected.then(|| {
                    rune::col()
                        .grow(1.0)
                        .min_w(0.0)
                        .gap(4.0)
                        .child(hint(model_label, kit))
                        .child(
                            rune::text(target.label())
                                .key("assistant-active-model")
                                .font(kit.ty.body_medium)
                                .color(kit.c.std_text)
                                .wrap(),
                        )
                }))
                .child(status),
        )
        .children(model_selected.then(|| hint(&model_footprint(target), kit)))
        .children(model_selected.then_some(context))
        .children(problem.map(|problem| hint(problem.status, kit)))
        .children(diagnostic.map(|message| hint(message, kit).role("status")))
}

pub(crate) fn setup_choice(state: &State) -> bool {
    state.supports_inbox_qa() && !state.inbox_qa.startup_seen
}

pub(crate) fn continue_setup(state: &mut State, tx: &mut Tx) {
    state.inbox_qa.startup_seen = true;
    tx.focus("startup-data-stage", true);
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
    let context_needed = "What would you like to work through? Point me to a source, or select a related job, news cycle, or status check.";
    let detail = match turn.conversation {
        Some(QaConversationReply::Acknowledgement) => "Of course. What else can I help with?",
        Some(QaConversationReply::Help) => {
            "Try “Is actor search ready?” or “How much memory is in use?” Select a job to ask about its status."
        }
        Some(QaConversationReply::ContextNeeded) => context_needed,
        Some(QaConversationReply::FollowUpNeedsTopic) => {
            "What would you like me to check again? Try “What’s the current memory usage?” or select the job you mean."
        }
        Some(QaConversationReply::ChooseSubject) => {
            "Which should I check first? Select a job or ask about one subject."
        }
        Some(QaConversationReply::RecallNeedsHistory) => "Ask me a question first, then I can recap our recent exchange.",
        _ => match turn.notice {
            Some(QaTurnNotice::Greeting) => {
                "Hi! How can I help? You can ask about actor search, memory usage, or running jobs."
            }
            Some(QaTurnNotice::OutOfScope) => context_needed,
            Some(QaTurnNotice::ObservationUnavailable) => {
                "I couldn’t get a current reading for that. Try again in a moment."
            }
            _ => fallback,
        },
    };
    (stage, detail)
}

fn answer_sources(turn_id: &str, citations: &[chat_ui::Citation], expanded: bool, kit: &Kit) -> El {
    let active_turn_id = turn_id.to_owned();
    let trigger = rune_kit::Button::new(format!("Sources ({})", citations.len()))
        .variant(rune_kit::ButtonVariant::Ghost)
        .icon(if expanded {
            kit.glyph.triangle_down
        } else {
            kit.glyph.triangle_right
        })
        .on_press_tx(move |state: &mut State, tx| {
            if state.inbox_qa_visible()
                && state
                    .displayed_inbox_qa_turn()
                    .is_some_and(|turn| turn.turn_id == active_turn_id)
            {
                let expanded = if state.browsed_inbox_qa_turn().is_some() {
                    &mut state.inbox_qa.history.sources_open
                } else {
                    &mut state.inbox_qa.sources_open
                };
                *expanded = !*expanded;
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
            chat_ui::citation_rows(turn_id, citations, kit)
                .key(SOURCES_CONTENT)
                .role("region")
                .label("Source citations")
        }))
}

fn context_dial(usage: Option<QaContextUsage>, kit: &Kit) -> El {
    let row = rune::row().key(CONTEXT_DIAL).gap(5.0).center_y();
    let usage = usage.filter(|usage| {
        usage.window_tokens > 0
            && usage.peak_input_tokens <= usage.window_tokens
            && (usage.model_windows == 0) == (usage.peak_input_tokens == 0)
    });
    let (row, fraction, tone, tokens, percentage) = if let Some(usage) = usage {
        let used = usage.peak_input_tokens;
        let capacity = usage.window_tokens;
        let fraction = f32::from(used) / f32::from(capacity);
        let percentage = format!("{:.0}%", fraction * 100.0);
        let tone = match fraction {
            _ if usage.model_windows == 0 => StatusTone::Neutral,
            value if value >= 0.95 => StatusTone::Err,
            value if value >= 0.85 => StatusTone::Warn,
            _ => StatusTone::Info,
        };
        let value = if usage.model_windows == 0 {
            format!("{used} of {capacity} tokens, {percentage}, model bypassed")
        } else {
            format!("{used} of {capacity} tokens, {percentage} peak usage")
        };
        (
            row.role("meter")
                .label("Model context window")
                .range_value(f64::from(used), 0.0, f64::from(capacity))
                .a11y_value(value),
            fraction,
            tone,
            format!("{used}/{capacity} tokens"),
            Some(percentage),
        )
    } else {
        (
            row.role("group")
                .label("Model context window, usage unavailable"),
            0.0,
            StatusTone::Neutral,
            "—".into(),
            None,
        )
    };
    let ink = tone.fg();
    let mut dial_kit = kit.clone();
    dial_kit.c.icon_muted = ink;
    row.child(
        ProgressCircle::new(fraction)
            .size(14.0)
            .build(&dial_kit)
            .a11y_hidden(),
    )
    .child(rune::icon(&icons::MODELS, 12.0).color(ink).a11y_hidden())
    .child(
        rune::text(tokens)
            .font(kit.ty.text_xsmall)
            .color(kit.c.text_muted)
            .a11y_hidden(),
    )
    .children(percentage.map(|text| {
        rune::text(text)
            .font(kit.ty.text_xsmall_medium)
            .color(ink)
            .a11y_hidden()
    }))
}

pub(crate) fn greeting(state: &State, kit: &Kit) -> Option<El> {
    state.can_send_inbox_qa().then(|| {
        let text = if state.permission(Permission::ReadWorkstationActivity) {
            "Hi! Ask me about workstation readiness, memory usage, or running jobs."
        } else {
            "Hi! Ask me about workstation readiness or memory usage."
        };
        chat_ui::greeting(GREETING, text, kit)
    })
}

fn history_arrow(
    qa: &InboxQa,
    direction: HistoryDirection,
    kit: &Kit,
    preferences: ViewPreferences,
) -> El {
    let (icon, label) = match direction {
        HistoryDirection::Previous => (&icons::CHEVRON_LEFT, "Previous exchange"),
        HistoryDirection::Next => (&icons::CHEVRON_RIGHT, "Next exchange"),
    };
    let control = IconButton::new(icon)
        .variant(IconButtonVariant::Ghost)
        .control_size(ControlSize::Large)
        .label(label)
        .disabled(!qa.can_browse(direction))
        .on_press_tx(move |state: &mut State, tx| state.browse_inbox_qa_history(direction, tx))
        .build(kit)
        .key(direction.key());
    material::glass_control(control, preferences)
}

pub(crate) fn exchange(state: &State, kit: &Kit, preferences: ViewPreferences) -> Option<El> {
    if !state.inbox_qa_visible() {
        return None;
    }
    let qa = &state.inbox_qa;
    let browsing = state.browsed_inbox_qa_turn().is_some();
    let pending = qa.pending.as_ref().filter(|_| !browsing);
    let (question, stage, answer, findings, request_id, turn_id, usage) =
        if let Some(pending) = pending {
            (
                pending.request.question.as_str(),
                chat_ui::TurnState::Queued,
                String::new(),
                &[][..],
                pending.request.request_id.as_str(),
                None,
                None,
            )
        } else if let Some(turn) = state.displayed_inbox_qa_turn() {
            let (stage, detail) = answer_state(turn);
            let draft = turn
                .provisional_reply
                .as_deref()
                .filter(|draft| turn.state == QaTurnState::Running && !draft.trim().is_empty());
            let stage = if draft.is_some() {
                chat_ui::TurnState::Streaming
            } else {
                stage
            };
            let reply = match turn.state {
                QaTurnState::Answered | QaTurnState::InsufficientContext => turn
                    .reply
                    .as_deref()
                    .map(str::trim)
                    .filter(|reply| !reply.is_empty()),
                _ => draft,
            };
            let answer = reply.map_or_else(
                || {
                    turn.findings
                        .iter()
                        .map(|finding| finding.text.as_str())
                        .chain((!detail.is_empty()).then_some(detail))
                        .collect::<Vec<_>>()
                        .join("\n")
                },
                str::to_owned,
            );
            (
                turn.question.as_str(),
                stage,
                answer,
                turn.findings.as_slice(),
                turn.request_id.as_str(),
                Some(turn.turn_id.as_str()),
                turn.context,
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
    let sources_open = if browsing {
        qa.history.sources_open
    } else {
        qa.sources_open
    };
    let sources = turn_id
        .filter(|_| !answer.citations.is_empty())
        .map(|turn_id| answer_sources(turn_id, &answer.citations, sources_open, kit));
    let live_region = if answer.state == chat_ui::TurnState::Streaming {
        LiveRegion::Off
    } else {
        LiveRegion::Polite
    };
    let bubbles = rune::col()
        .w_full()
        .gap(8.0)
        .instant_changes()
        .transition(rune::Transition::none())
        .child(chat_ui::bubble(&question, kit))
        .child(
            chat_ui::bubble_with_sources(&answer, sources, kit)
                .key("inbox-qa-live-answer")
                .live_region(live_region),
        )
        .child(context_dial(usage, kit))
        .children(state.canonical_related_activity().map(|related| {
            crate::activity_triage::result_view(state, related, None, kit, preferences)
        }));
    // Admission and streaming keep the same request key as the queued draft.
    let turn = crate::inbox::enter_from_below(
        rune::col()
            .key(format!("inbox-qa-turn-{request_id}"))
            .w_full()
            .child(bubbles),
        preferences,
    );
    let show_arrows = state.supports_inbox_qa_history() && qa.earlier_turns().next().is_some();
    let transcript = rune::row()
        .key("inbox-qa-history")
        .w_full()
        .items_end()
        .gap(6.0)
        .children(
            show_arrows.then(|| history_arrow(qa, HistoryDirection::Previous, kit, preferences)),
        )
        .child(turn.grow(1.0).min_w(0.0))
        .children(show_arrows.then(|| history_arrow(qa, HistoryDirection::Next, kit, preferences)));
    Some(
        rune::col()
            .key("inbox-qa-exchange")
            .w_full()
            .min_w(0.0)
            .child(transcript),
    )
}

pub(crate) fn footer(state: &State, kit: &Kit, preferences: ViewPreferences) -> Option<El> {
    state
        .is_local_backend()
        .then(|| footer_content(state, kit, preferences))
}

fn footer_content(state: &State, kit: &Kit, preferences: ViewPreferences) -> El {
    let progress = if state.inbox_qa.pending_inference == Some(true) {
        Some("Starting backend inference")
    } else {
        startup_label(state)
    };
    let problem = embedded_problem(state);
    let triage = state.supports_workstation_triage() && state.inbox_qa_visible();
    if !triage {
        if let Some(label) = progress {
            return footer_layer(
                hint(label, kit)
                    .key("inbox-qa-startup-status")
                    .role("status")
                    .live_region(LiveRegion::Polite),
                preferences,
            );
        }
        if let Some(problem) = problem {
            return footer_layer(problem_content(state, problem, kit), preferences);
        }
    }
    if !state.inbox_qa_visible() {
        return nudge(state, kit, preferences);
    }
    let qa = &state.inbox_qa;
    let phase = state.inbox_qa_settings().map(|settings| settings.phase);
    if phase != Some(QaPhase::Ready) && !triage {
        let label = if matches!(phase, Some(QaPhase::Off | QaPhase::Starting)) {
            "Preparing backend inference…"
        } else {
            UNAVAILABLE_NUDGE
        };
        return footer_layer(hint(label, kit).role("status"), preferences);
    }
    if !state.permission(Permission::SubmitInteractive) {
        return footer_layer(hint("This registration can view answers. Sending a question requires interactive access.", kit), preferences);
    }
    let input = rune_kit::TextArea::new(&qa.draft)
        .label("Question for the Inbox assistant")
        .placeholder("Ask a question")
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
            ActionButton::new(format!(
                "Remove {} source",
                citation_kind(source.kind).to_lowercase()
            ))
            .key("inbox-qa-clear-source")
            .finish(MetalFinish::Gunmetal)
            .disabled(qa.pending.is_some())
            .on_press(|state: &mut State| state.inbox_qa.source = None)
            .build(kit, preferences, None),
        );
    } else {
        let current = assistant::current_context(state);
        if let Some(source) = assistant::source_for(&current) {
            row = row.child(
                ActionButton::new(format!("Include {}", current.kind.to_lowercase()))
                    .key("inbox-qa-pin-source")
                    .finish(MetalFinish::Gunmetal)
                    .disabled(qa.pending.is_some())
                    .on_press(move |state: &mut State| state.inbox_qa.source = Some(source.clone()))
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
    if phase != Some(QaPhase::Ready) {
        let reason = progress
            .or_else(|| problem.map(|problem| problem.status))
            .unwrap_or("Language model unavailable");
        content = content.child(
            hint(
                &format!("{reason}. Direct facts and verified links are available."),
                kit,
            )
            .key("inbox-qa-limited")
            .role("status")
            .live_region(LiveRegion::Polite),
        );
    }
    if let Some(status) = qa.status {
        content = content.child(hint(status, kit).role("status"));
    }
    if qa.active().is_some() && state.browsed_inbox_qa_turn().is_some() {
        content = content.child(hint("A newer exchange is running.", kit).role("status"));
    }
    footer_layer(content.child(input).child(row), preferences)
}

fn nudge(state: &State, kit: &Kit, preferences: ViewPreferences) -> El {
    let text = if !state.connected() {
        "Reconnect the selected backend to check inference."
    } else if !state.supports_inbox_qa() {
        "Backend inference is unavailable for this selection."
    } else if state.inbox_qa.pending_inference == Some(false)
        || state
            .inbox_qa_settings()
            .is_some_and(|settings| settings.phase == QaPhase::Stopping)
    {
        "Stopping"
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
        OPERATOR_NUDGE
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
    use fleetworks_contracts::chat::ChatSourceReference;
    use fleetworks_contracts::chat::QaFinding;
    use rune::Env;
    use rune::Ime;
    use rune::Mods;
    use rune::Runtime;

    use super::*;

    const FINDING: &str = "Actor search is ready.";
    const FIXTURE_CITATION: &str = "chat-citation-fixture-turn-0";

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
        qa_drawer(size, Some(turn(QaTurnState::Running, 1)))
    }

    fn qa_drawer(size: (f32, f32), latest: Option<QaTurn>) -> Runtime<State> {
        let mut state = crate::tests::registered_fixture();
        state.page = crate::app::Page::Workstation;
        state.backend_kind = BackendKind::Local;
        state.connection.origin = TextEdit::new("local://embedded");
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
        if latest.is_none() {
            state.seed_inbox_qa_overflow_fixture();
        } else {
            state.seed_inbox_qa_drawer_fixture();
        }
        state.seed_inbox_qa_fixture(QaSettings::new(true, QaPhase::Ready, 1, 1, true), latest);
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
    fn empty_transcript_keeps_the_assistant_greeting_visible_with_notification_history() {
        for size in [(640.0, 480.0), (1280.0, 800.0)] {
            let mut runtime = qa_drawer(size, None);
            let tree = runtime.accessibility_json();
            assert_eq!(accessibility_nodes_with_key(&tree, GREETING), 1);
            let rendered = tree.to_string();
            assert!(rendered.contains("Assistant message"));
            assert!(!rendered.contains(r#""name":"Assistant""#));
            assert!(rendered.contains("workstation readiness, memory usage, or running jobs"));
            assert!(runtime.find(COMPOSER).is_some());
            assert!(runtime.find(SOURCES_TOGGLE).is_none());
            assert!(runtime.find(CONTEXT_DIAL).is_none());
            assert!(runtime.state.inbox_qa.latest.is_none());
            assert!(runtime.state.inbox_qa.pending.is_none());
            let (greeting, _) = runtime.presented(GREETING).unwrap();
            let (scroll, _) = runtime.presented(crate::inbox::SCROLL).unwrap();
            assert!(greeting[1] >= scroll[1] - 0.5);
            assert!(greeting[1] + greeting[3] <= scroll[1] + scroll[3] + 0.5);
            if size.1 == 480.0 {
                let (first_card, _) = runtime.presented("inbox-group-connection-toggle").unwrap();
                assert!(first_card[1] < scroll[1]);
            }

            runtime.update(|state, _| {
                let capabilities = state.connection.capabilities.as_mut().unwrap();
                capabilities.features.push(QA_TRIAGE_FEATURE.into());
                capabilities
                    .permissions
                    .retain(|permission| *permission != Permission::ReadWorkstationActivity);
            });
            runtime.frame(0.625);
            let limited_prompt = runtime.accessibility_json().to_string();
            assert!(limited_prompt.contains("workstation readiness or memory usage"));
            assert!(!limited_prompt.contains("or running jobs"));

            runtime.update(|state, _| state.inbox_qa.request_resume());
            runtime.frame(0.75);
            assert!(runtime.find(GREETING).is_none());

            runtime.update(|state, tx| {
                let context = MutationContext::new(state.registration.context.as_ref().unwrap());
                let request = QaTurnRequest::new(
                    "fixture-queued".into(),
                    context,
                    "What is the current memory usage?".into(),
                    None,
                )
                .unwrap();
                state.inbox_qa.resume = false;
                state.inbox_qa.pending = Some(Pending { request });
                tx.scroll_to_bottom_instant(crate::inbox::SCROLL);
            });
            runtime.frame(1.0);
            assert!(runtime.find(GREETING).is_none());
            assert!(runtime.find(REPLY).is_some());
            assert_eq!(runtime.scroll_at_bottom(crate::inbox::SCROLL), Some(true));

            runtime.update(|state, _| {
                state.inbox_qa.pending = None;
                state.inbox_qa.adopt(Some(turn(QaTurnState::Answered, 3)));
            });
            runtime.frame(1.25);
            assert!(runtime.find(GREETING).is_none());
            assert!(runtime.accessibility_json().to_string().contains(FINDING));
        }
    }

    #[test]
    fn composer_can_include_the_selected_readiness_check() {
        let mut runtime = active_drawer((640.0, 480.0));
        let key = "inbox-qa-pin-source";
        assert!(runtime.find(key).is_none());

        runtime.update(|state, _| state.selected_readiness = Some("actor_search".into()));
        runtime.frame(1.0);
        assert!(runtime.find(key).is_some());
        assert!(runtime
            .accessibility_json()
            .to_string()
            .contains("Include readiness"));
    }

    #[test]
    fn assistant_controls_follow_embedded_inference_and_model_readiness() {
        let mut runtime = active_drawer((1280.0, 800.0));
        runtime.update(|state, _| {
            state.inbox.close();
            let mut answered = turn(QaTurnState::Answered, 3);
            answered.context = Some(QaContextUsage::new(512, 128, 1));
            state.inbox_qa.adopt(Some(answered));
            state.fixture_show_assistant_settings();
        });
        runtime.frame(1.0);
        assert!(runtime.find("settings-category-assistant").is_some());
        let tree = runtime.accessibility_json().to_string();
        let model = QaModelTarget::default();
        assert!(tree.contains(model.label()));
        assert!(tree.contains(&model_footprint(model)));
        assert!(tree.contains("Active model"));
        assert!(tree.contains("128 of 512 tokens, 25% peak usage"));
        assert!(runtime.find("assistant-context-usage").is_some());
        assert!(!tree.contains("Model Target"));
        assert!(runtime.find("assistant-generator-preference").is_none());
        assert!(runtime.find("assistant-decision-preference").is_none());
        assert!(runtime.find("assistant-model-status").is_some());

        runtime.update(|state, _| {
            let mut next = QaSettings::new(true, QaPhase::Starting, 2, 2, true);
            next.model_target = QaModelTarget::FlanT5Base;
            next.startup_stage = Some(QaStartupStage::DownloadingPack);
            state.inbox_qa.apply_settings(next, 1.5);
        });
        runtime.frame(1.5);
        let tree = runtime.accessibility_json().to_string();
        assert!(tree.contains("Downloading QA pack"));
        assert!(tree.contains(QaModelTarget::FlanT5Base.label()));
        assert!(tree.contains("Selected model"));
        assert!(tree.contains("512 tokens · No measured usage"));

        runtime.update(|state, _| {
            state.settings_open = false;
            state.seed_inbox_qa_drawer_fixture();
        });
        runtime.frame(2.0);
        assert!(runtime.find("inbox-qa-startup-status").is_some());
        assert!(runtime.find(COMPOSER).is_none());

        runtime.update(|state, _| {
            state.inbox.close();
            state.fixture_show_assistant_settings();
            state
                .inbox_qa
                .apply_settings(QaSettings::new(false, QaPhase::Off, 3, 3, true), 2.5);
        });
        runtime.frame(2.5);
        assert!(runtime.find("settings-category-assistant").is_none());
        assert!(runtime.find("assistant-active-model").is_none());

        runtime.update(|state, _| {
            state.backend_kind = BackendKind::Remote;
            state.settings_open = false;
            state.seed_inbox_qa_drawer_fixture();
            state.seed_inbox_qa_fixture(
                QaSettings::new(true, QaPhase::Ready, 1, 1, true),
                Some(turn(QaTurnState::Answered, 3)),
            );
        });
        runtime.frame(3.0);
        assert!(runtime.find("inbox-group-connection-toggle").is_some());
        assert!(runtime.find("inbox-qa-footer").is_none());
        assert!(runtime.find("inbox-qa-exchange").is_none());
    }

    fn model_settings(
        generator: QaGeneratorPreference,
        decision: QaDecisionPreference,
        revision: u64,
        generation: u64,
    ) -> QaSettings {
        let mut settings = QaSettings::new(true, QaPhase::Ready, revision, generation, true);
        settings.model_target = generator_target(generator).unwrap_or(QaModelTarget::FlanT5Small);
        settings.generator = Some(QaGeneratorSettings::new(
            generator,
            Some(settings.model_target),
            QaPhase::Ready,
            None,
        ));
        settings.decision = Some(QaDecisionSettings::new(
            decision,
            QaDecisionPreference::RulesOnly,
            QaPhase::Off,
            (decision != QaDecisionPreference::RulesOnly)
                .then_some(QaDecisionAvailabilityReason::AwaitingQualification),
        ));
        settings
    }

    fn model_settings_runtime() -> Runtime<State> {
        let mut runtime = active_drawer((640.0, 480.0));
        runtime.update(|state, _| {
            state.inbox.close();
            let capabilities = state
                .connection
                .capabilities
                .as_mut()
                .expect("capabilities");
            capabilities.features.push(QA_MODEL_SETTINGS_FEATURE.into());
            let settings = model_settings(
                QaGeneratorPreference::Automatic,
                QaDecisionPreference::RulesOnly,
                10,
                7,
            );
            let mut answered = turn(QaTurnState::Answered, 3);
            answered.context = Some(QaContextUsage::new(512, 128, 1));
            answered.model_target = Some(settings.model_target);
            answered.model_generation = Some(settings.generation);
            state.seed_inbox_qa_fixture(settings, Some(answered));
            state.fixture = false;
            assert!(state.sync_inbox_qa_binding());
            state.fixture = true;
            state.fixture_show_assistant_settings();
        });
        runtime.frame(1.0);
        runtime.frame(1.25);
        runtime
    }

    fn choose_model_setting(runtime: &mut Runtime<State>, label: &str, now: f64) -> String {
        let option = runtime
            .elements()
            .into_iter()
            .find(|element| element.role == "radio" && element.label.as_deref() == Some(label))
            .unwrap_or_else(|| panic!("missing model setting {label}"));
        let path = option.path;
        let key = option.key.expect("model setting key");
        runtime.update(move |_, tx| {
            tx.scroll_into_view(key.clone());
            tx.focus(key, true);
        });
        runtime.frame(now);
        let control = runtime
            .elements()
            .into_iter()
            .find(|element| element.path == path)
            .unwrap_or_else(|| panic!("missing model setting {label} after scrolling"));
        assert!(
            control.focused && control.focus_visible,
            "model choice {label} must have keyboard focus"
        );
        // Rune subtracts the clipped edges to report width and height.
        assert!(
            control
                .visible
                .into_iter()
                .zip(control.rect)
                .all(|(visible, actual)| (visible - actual).abs() < 0.001),
            "model choice {label} must be fully visible: {:?} of {:?}",
            control.visible,
            control.rect
        );
        runtime.key(KeyEvent {
            key: Key::Text(" ".into()),
            mods: Mods::default(),
        });
        runtime.frame(now + 0.1);
        path
    }

    fn model_settings_reply(runtime: &mut Runtime<State>, reply: Reply) {
        runtime.update(move |state, tx| {
            let ticket = Ticket {
                epoch: state.connection.epoch,
                origin_revision: state.origin_revision,
                request_revision: state.inbox_qa.request_revision,
            };
            state.receive_inbox_qa(ticket, reply, tx);
        });
    }

    #[test]
    fn assistant_preferences_save_independently_and_keep_focus_at_minimum_width() {
        let mut runtime = model_settings_runtime();
        let choices: Vec<_> = runtime
            .elements()
            .into_iter()
            .filter(|element| element.role == "radio")
            .filter_map(|element| element.label)
            .collect();
        assert_eq!(
            choices,
            [
                "Automatic",
                "FLAN-T5 Small · 308 MB",
                "FLAN-T5 Base · 990 MB",
                "Rules only",
                "Julia-1 Q8 · 168 MB",
                "vLLM d3-edge Q8 · 806 MB",
            ]
        );
        assert!(runtime.find("assistant-context-usage").is_some());

        let generator = choose_model_setting(&mut runtime, "FLAN-T5 Base · 990 MB", 1.5);
        let update = runtime
            .state
            .inbox_qa
            .captured_update
            .as_ref()
            .expect("generator update");
        assert_eq!(update.expected_revision, 10);
        assert_eq!(update.enabled, None);
        assert_eq!(
            update.generator_preference,
            Some(QaGeneratorPreference::FlanT5Base)
        );
        assert_eq!(update.decision_preference, None);
        assert!(runtime
            .accessibility_json()
            .to_string()
            .contains("Saving preference"));

        model_settings_reply(
            &mut runtime,
            Reply::Update(Ok(model_settings(
                QaGeneratorPreference::FlanT5Base,
                QaDecisionPreference::RulesOnly,
                11,
                8,
            ))),
        );
        runtime.frame(1.8);
        assert!(runtime
            .elements()
            .into_iter()
            .any(|element| { element.path == generator && element.focused }));
        assert!(runtime.find("assistant-context-usage").is_none());
        assert!(runtime
            .state
            .inbox_qa
            .latest
            .as_ref()
            .is_some_and(|turn| { turn.state == QaTurnState::Answered && turn.context.is_some() }));

        runtime.update(|state, _| {
            let mut answered = turn(QaTurnState::Answered, 4);
            answered.context = Some(QaContextUsage::new(512, 128, 1));
            answered.model_target = Some(QaModelTarget::FlanT5Base);
            answered.model_generation = Some(8);
            state.inbox_qa.adopt(Some(answered));
        });
        runtime.frame(1.9);
        assert!(runtime.find("assistant-context-usage").is_some());

        let decision = choose_model_setting(&mut runtime, "Julia-1 Q8 · 168 MB", 2.0);
        let update = runtime
            .state
            .inbox_qa
            .captured_update
            .as_ref()
            .expect("decision update");
        assert_eq!(update.expected_revision, 11);
        assert_eq!(update.enabled, None);
        assert_eq!(update.generator_preference, None);
        assert_eq!(
            update.decision_preference,
            Some(QaDecisionPreference::Julia1Q8)
        );

        model_settings_reply(
            &mut runtime,
            Reply::Update(Ok(model_settings(
                QaGeneratorPreference::FlanT5Base,
                QaDecisionPreference::Julia1Q8,
                12,
                9,
            ))),
        );
        runtime.frame(2.3);
        assert!(runtime
            .elements()
            .into_iter()
            .any(|element| { element.path == decision && element.focused }));
        assert!(runtime.find("assistant-context-usage").is_none());
        let tree = runtime.accessibility_json().to_string();
        assert!(tree.contains("Preferred · Julia-1 Q8"));
        assert!(tree.contains("In use · Rules only"));
        assert!(tree.contains("Awaiting qualification on this platform"));

        runtime.update(|state, _| {
            state
                .inbox_qa
                .settings
                .as_mut()
                .expect("settings")
                .can_manage = false;
        });
        runtime.frame(2.5);
        assert!(runtime
            .accessibility_json()
            .to_string()
            .contains("Only a workstation manager can change model preferences."));
        assert!(runtime
            .elements()
            .into_iter()
            .any(|element| { element.path == decision && !element.focusable }));
    }

    #[test]
    fn assistant_preference_conflict_refreshes_and_old_backend_replies_are_fenced() {
        let mut runtime = model_settings_runtime();
        choose_model_setting(&mut runtime, "FLAN-T5 Small · 308 MB", 1.5);
        model_settings_reply(&mut runtime, Reply::Update(Err(ClientError::Status(409))));
        runtime.frame(1.8);
        assert!(runtime.state.inbox_qa.settings_refresh_required);
        assert!(runtime
            .accessibility_json()
            .to_string()
            .contains("Settings changed. Review the current preference"));
        choose_model_setting(&mut runtime, "FLAN-T5 Base · 990 MB", 2.0);
        let update = runtime
            .state
            .inbox_qa
            .captured_update
            .as_ref()
            .expect("original update");
        assert_eq!(
            update.generator_preference,
            Some(QaGeneratorPreference::FlanT5Small)
        );
        model_settings_reply(
            &mut runtime,
            Reply::Settings(Ok(model_settings(
                QaGeneratorPreference::Automatic,
                QaDecisionPreference::RulesOnly,
                14,
                7,
            ))),
        );
        runtime.frame(2.3);
        assert!(!runtime.state.inbox_qa.settings_refresh_required);
        choose_model_setting(&mut runtime, "vLLM d3-edge Q8 · 806 MB", 2.5);
        let update = runtime
            .state
            .inbox_qa
            .captured_update
            .as_ref()
            .expect("current update");
        assert_eq!(update.expected_revision, 14);
        assert_eq!(
            update.decision_preference,
            Some(QaDecisionPreference::VllmD3EdgeQ8)
        );
        runtime.update(|state, tx| {
            let old_ticket = Ticket {
                epoch: state.connection.epoch,
                origin_revision: state.origin_revision,
                request_revision: state.inbox_qa.request_revision,
            };
            state.backend_kind = BackendKind::Remote;
            state.inbox_qa.detach();
            state.receive_inbox_qa(
                old_ticket,
                Reply::Update(Ok(model_settings(
                    QaGeneratorPreference::Automatic,
                    QaDecisionPreference::VllmD3EdgeQ8,
                    15,
                    8,
                ))),
                tx,
            );
        });
        runtime.frame(2.8);
        assert!(runtime.find("settings-category-assistant").is_none());
        assert_eq!(runtime.state.inbox_qa.settings.unwrap().revision, 14);
    }

    #[test]
    fn workstation_assistant_shortcut_returns_focus_and_preserves_checks_when_off() {
        const SHORTCUT: &str = crate::views::WORKSTATION_ASSISTANT;
        let mut runtime = active_drawer((640.0, 480.0));
        runtime.update(|state, _| state.inbox.close());
        runtime.frame(1.0);

        press_disclosure(&mut runtime, SHORTCUT, 1.5);
        assert!(runtime.find("inbox-layer").is_some());
        assert!(runtime.find("inbox-filter").is_some());
        runtime.key(KeyEvent {
            key: Key::Escape,
            mods: Mods::default(),
        });
        runtime.frame(2.0);
        assert!(runtime.find("inbox-layer").is_none());
        assert!(runtime
            .elements()
            .iter()
            .any(|element| element.key.as_deref() == Some(SHORTCUT) && element.focused));

        runtime.update(|state, _| {
            state
                .inbox_qa
                .apply_settings(QaSettings::new(false, QaPhase::Off, 2, 2, true), 2.5);
        });
        runtime.frame(2.5);
        assert!(runtime.find(SHORTCUT).is_none());
        assert!(runtime.find("workstation-health").is_some());
        assert!(runtime.find("status-checks-toggle/trigger").is_some());
        assert!(runtime.find("workstation-diagnostics/trigger").is_some());
    }

    #[test]
    fn provisional_text_recovers_in_one_bubble_and_yields_to_the_canonical_turn() {
        let mut runtime = active_drawer((640.0, 480.0));
        let draft = "Provisional model draft";
        runtime.update(move |state, _| {
            let mut running = turn(QaTurnState::Running, 1);
            running.cursor = 2;
            running.provisional_reply = Some(draft.into());
            state.inbox_qa.adopt(Some(running));
        });
        runtime.frame(1.0);
        let tree = runtime.accessibility_json();
        assert_eq!(accessibility_nodes_with_key(&tree, REPLY), 1);
        let rendered = tree.to_string();
        assert!(rendered.contains(draft));
        assert!(rendered.contains("Draft · unverified"));
        assert!(runtime.find("inbox-qa-stop").is_some());

        runtime.update(move |state, _| {
            let mut recovered = turn(QaTurnState::Running, 1);
            recovered.cursor = 4;
            recovered.provisional_reply = Some(format!("{draft} from a recovered page"));
            state.inbox_qa.adopt(Some(recovered));
        });
        runtime.frame(1.25);
        assert!(runtime
            .accessibility_json()
            .to_string()
            .contains("from a recovered page"));

        runtime.update(|state, _| {
            let mut answer = turn(QaTurnState::Answered, 5);
            answer.reply = Some(FINDING.into());
            state.inbox_qa.adopt(Some(answer));
        });
        runtime.frame(1.5);
        let tree = runtime.accessibility_json();
        assert_eq!(accessibility_nodes_with_key(&tree, REPLY), 1);
        let rendered = tree.to_string();
        assert!(rendered.contains(FINDING));
        assert!(!rendered.contains(draft));
        assert!(!rendered.contains("Draft · unverified"));
        assert!(runtime.find(SOURCES_TOGGLE).is_some());
        assert!(runtime.find("inbox-qa-stop").is_none());
    }

    #[test]
    fn context_dial_uses_the_response_usage_and_distinguishes_bypass_from_unknown() {
        let mut runtime = active_drawer((640.0, 480.0));
        let assert_unknown = |runtime: &Runtime<State>| {
            let tree = runtime.accessibility_json();
            assert_eq!(accessibility_nodes_with_key(&tree, CONTEXT_DIAL), 1);
            let rendered = tree.to_string();
            assert!(rendered.contains("Model context window, usage unavailable"));
            assert!(!rendered.contains("model bypassed"));
        };
        assert_unknown(&runtime);

        runtime.update(|state, _| {
            let mut modeled = turn(QaTurnState::Answered, 2);
            modeled.reply = Some(format!("Here are the current checks. {FINDING}"));
            modeled.notice = Some(QaTurnNotice::Greeting);
            modeled.conversation = Some(QaConversationReply::Help);
            modeled.context = Some(QaContextUsage::new(
                QaModelTarget::FlanT5Small.input_tokens(),
                448,
                1,
            ));
            state.inbox_qa.adopt(Some(modeled));
        });
        runtime.frame(1.0);
        let tree = runtime.accessibility_json();
        assert_eq!(accessibility_nodes_with_key(&tree, REPLY), 1);
        assert_eq!(accessibility_nodes_with_key(&tree, CONTEXT_DIAL), 1);
        let rendered = tree.to_string();
        assert!(rendered.contains("Here are the current checks."));
        assert!(rendered.contains(FINDING));
        assert!(!rendered.contains("How can I help?"));
        assert!(!rendered.contains("Select a job to ask about its status"));
        assert!(runtime.find(SOURCES_TOGGLE).is_some());
        assert!(runtime.find(FIXTURE_CITATION).is_none());
        assert!(rendered.contains("448 of 512 tokens, 88% peak usage"));
        assert!(runtime.elements().iter().any(|element| {
            !element.exiting && element.text.as_deref() == Some("448/512 tokens")
        }));
        let (reply, _) = runtime.presented(REPLY).unwrap();
        let (dial, _) = runtime.presented(CONTEXT_DIAL).unwrap();
        assert!(dial[1] >= reply[1] + reply[3]);

        runtime.update(|state, _| {
            let mut direct = turn(QaTurnState::Answered, 2);
            direct.turn_id = "fixture-direct".into();
            direct.request_id = "fixture-direct-request".into();
            direct.context = Some(QaContextUsage::new(
                QaModelTarget::FlanT5Base.input_tokens(),
                0,
                0,
            ));
            state.inbox_qa.adopt(Some(direct));
        });
        runtime.frame(1.5);
        let rendered = runtime.accessibility_json().to_string();
        assert!(rendered.contains("0 of 512 tokens, 0%, model bypassed"));
        assert!(!rendered.contains("448 of 512 tokens"));

        runtime.update(|state, _| {
            let mut stopped = turn(QaTurnState::Canceled, 3);
            stopped.turn_id = "fixture-stopped".into();
            stopped.request_id = "fixture-stopped-request".into();
            state.inbox_qa.adopt(Some(stopped));
        });
        runtime.frame(2.0);
        assert_unknown(&runtime);
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
            let (notifications, _) = runtime.presented("inbox-list").unwrap();
            let (divider, _) = runtime.presented("inbox-chat-divider").unwrap();
            assert!(notifications[1] + notifications[3] < divider[1]);
            assert!(divider[2] > 0.0 && divider[3] > 0.0);
            assert!(divider[1] + divider[3] < exchange[1]);
            assert!(runtime.find("inbox-qa-stop").is_some());
            assert!(runtime.find(SOURCES_TOGGLE).is_none());
            let unread = runtime.state.inbox.unread();

            runtime.update(|state, _| {
                state.inbox_qa.adopt(Some(turn(QaTurnState::Running, 2)));
            });
            runtime.frame(1.0);
            assert!(runtime.find(FIXTURE_CITATION).is_none());
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
                .find(|el| el.key.as_deref() == Some(FIXTURE_CITATION))
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
            assert!(runtime.find(FIXTURE_CITATION).is_some());
            let tree = runtime.accessibility_json();
            assert_eq!(accessibility_nodes_with_key(&tree, REPLY), 1);
            let rendered = tree.to_string();
            assert_eq!(rendered.matches("Assistant message").count(), 1);
            assert!(rendered.contains("live-polite"));
            assert!(rendered.contains(FINDING) && rendered.contains("Stopped"));
            assert_eq!(runtime.state.inbox.unread(), unread);

            press_disclosure(&mut runtime, SOURCES_TOGGLE, 2.0);
            assert!(runtime.find(FIXTURE_CITATION).is_none());
            press_disclosure(&mut runtime, SOURCES_TOGGLE, 2.5);
            assert!(runtime.find(FIXTURE_CITATION).is_some());
            runtime.update(|state, _| {
                let mut next = turn(QaTurnState::Answered, 2);
                next.turn_id = "fixture-next-turn".into();
                next.request_id = "fixture-next-request".into();
                state.inbox_qa.adopt(Some(next));
            });
            runtime.frame(3.0);
            assert!(runtime.find(SOURCES_TOGGLE).is_some());
            let next_citation = "chat-citation-fixture-next-turn-0";
            assert!(runtime.find(next_citation).is_none());
            press_disclosure(&mut runtime, SOURCES_TOGGLE, 3.1);
            assert!(runtime.find(next_citation).is_some());
            assert!(runtime.find(FIXTURE_CITATION).is_none());

            runtime.update(|state, _| {
                state
                    .inbox_qa
                    .apply_settings(QaSettings::new(false, QaPhase::Off, 2, 2, false), 3.5);
            });
            runtime.frame(3.5);
            assert!(runtime.find(COMPOSER).is_none());
            assert!(runtime.find(REPLY).is_none());
            assert!(runtime
                .accessibility_json()
                .to_string()
                .contains(OPERATOR_NUDGE));
            assert_eq!(runtime.state.inbox.unread(), unread);
        }
    }

    #[test]
    fn transcript_arrows_browse_retained_turns_without_changing_the_live_exchange() {
        const OLDER_QUESTION: &str = "Which earlier check finished?";
        const OLDER_CITATION: &str = "chat-citation-older-turn-0";
        for size in [(640.0, 480.0), (1280.0, 800.0)] {
            let mut runtime = active_drawer(size);
            let mut older = turn(QaTurnState::Answered, 3);
            older.turn_id = "older-turn".into();
            older.request_id = "older-request".into();
            older.question = OLDER_QUESTION.into();
            runtime.update(move |state, tx| {
                state.seed_inbox_qa_history_fixture(vec![
                    turn(QaTurnState::Running, 1),
                    older.clone(),
                    older,
                ]);
                state.inbox_qa.draft = TextEdit::new("Unsent follow-up");
                tx.scroll_to_bottom_instant(crate::inbox::SCROLL);
            });
            runtime.frame(0.75);
            let elements = runtime.elements();
            let previous = elements
                .iter()
                .find(|element| element.key.as_deref() == Some(HISTORY_PREVIOUS))
                .expect("left transcript control");
            let next = elements
                .iter()
                .find(|element| element.key.as_deref() == Some(HISTORY_NEXT))
                .expect("right transcript control");
            assert_eq!(previous.label.as_deref(), Some("Previous exchange"));
            assert_eq!(next.label.as_deref(), Some("Next exchange"));
            assert!(previous.focusable && !next.focusable);
            let (turn_rect, _) = runtime.presented("inbox-qa-turn-fixture-request").unwrap();
            assert!(previous.rect[0] + previous.rect[2] <= turn_rect[0]);
            assert!(turn_rect[0] + turn_rect[2] <= next.rect[0]);
            let (x, y) = runtime.find(HISTORY_PREVIOUS).unwrap();
            let (viewport, _) = runtime.presented(crate::inbox::SCROLL).unwrap();
            assert!(y >= viewport[1] && y < viewport[1] + viewport[3]);
            runtime.pointer_down(x, y);
            runtime.pointer_up(x, y);
            runtime.frame(1.0);
            assert!(runtime
                .accessibility_json()
                .to_string()
                .contains(OLDER_QUESTION));
            assert!(runtime.find("inbox-qa-stop").is_some());
            assert!(runtime.elements().iter().any(|element| {
                element.key.as_deref() == Some(HISTORY_PREVIOUS) && !element.focusable
            }));
            press_disclosure(&mut runtime, SOURCES_TOGGLE, 1.1);
            assert!(runtime.find(OLDER_CITATION).is_some());

            runtime.update(|state, _| {
                state.inbox_qa.adopt(Some(turn(QaTurnState::Answered, 3)));
            });
            runtime.frame(1.5);
            assert!(runtime
                .accessibility_json()
                .to_string()
                .contains(OLDER_QUESTION));
            assert!(runtime.find(OLDER_CITATION).is_some());
            assert!(runtime.find("inbox-qa-stop").is_none());
            press_disclosure(&mut runtime, HISTORY_NEXT, 1.75);
            assert!(!runtime
                .accessibility_json()
                .to_string()
                .contains(OLDER_QUESTION));
            assert!(runtime.find(OLDER_CITATION).is_none());
            assert_eq!(runtime.state.inbox_qa.draft.text, "Unsent follow-up");
            assert!(runtime.elements().iter().any(|element| {
                element.key.as_deref() == Some(HISTORY_NEXT) && !element.focusable
            }));
        }
    }

    #[test]
    fn history_reconciliation_removes_expired_turns_and_fences_a_detached_reply() {
        let mut runtime = qa_drawer((640.0, 480.0), Some(turn(QaTurnState::Answered, 3)));
        let mut older = turn(QaTurnState::Answered, 3);
        older.turn_id = "old-turn".into();
        older.request_id = "old-request".into();
        runtime.update(|state, _| {
            state.fixture = false;
            assert!(state.sync_inbox_qa_binding());
            state.fixture = true;
        });
        runtime.update(|state, tx| {
            state.seed_inbox_qa_history_fixture(vec![older.clone()]);
            state.browse_inbox_qa_history(HistoryDirection::Previous, tx);
            assert!(state.browsed_inbox_qa_turn().is_some());
            let ticket = Ticket {
                epoch: state.connection.epoch,
                origin_revision: state.origin_revision,
                request_revision: state.inbox_qa.request_revision,
            };
            state.receive_inbox_qa(
                ticket,
                Reply::History(Ok(vec![turn(QaTurnState::Answered, 3)])),
                tx,
            );
            assert!(state.browsed_inbox_qa_turn().is_none());
            assert!(!state.inbox_qa.can_browse(HistoryDirection::Previous));
            state.inbox_qa.detach();
            state.receive_inbox_qa(ticket, Reply::History(Ok(vec![older])), tx);
            assert!(state.inbox_qa.history.turns.is_empty());
        });
    }

    #[test]
    fn informational_notices_keep_the_answer_and_composer_available() {
        let mut runtime = active_drawer((640.0, 480.0));
        for (index, (notice, conversation, copy)) in [
            (QaTurnNotice::Greeting, None, "How can I help?"),
            (
                QaTurnNotice::OutOfScope,
                Some(QaConversationReply::Acknowledgement),
                "What else can I help with?",
            ),
            (
                QaTurnNotice::OutOfScope,
                Some(QaConversationReply::Help),
                "Select a job to ask about its status",
            ),
            (
                QaTurnNotice::OutOfScope,
                None,
                "What would you like to work through?",
            ),
            (
                QaTurnNotice::OutOfScope,
                Some(QaConversationReply::ContextNeeded),
                "select a related job, news cycle, or status check",
            ),
            (
                QaTurnNotice::OutOfScope,
                Some(QaConversationReply::FollowUpNeedsTopic),
                "What’s the current memory usage?",
            ),
            (
                QaTurnNotice::OutOfScope,
                Some(QaConversationReply::ChooseSubject),
                "Which should I check first?",
            ),
            (
                QaTurnNotice::ObservationUnavailable,
                None,
                "Try again in a moment.",
            ),
            (
                QaTurnNotice::ObservationUnavailable,
                Some(QaConversationReply::Unknown),
                "Try again in a moment.",
            ),
        ]
        .into_iter()
        .enumerate()
        {
            runtime.update(move |state, _| {
                let mut reply = turn(QaTurnState::InsufficientContext, 1);
                reply.turn_id = format!("fixture-notice-{index}");
                reply.notice = Some(notice);
                reply.conversation = conversation;
                state.inbox_qa.adopt(Some(reply));
            });
            runtime.frame(1.0 + index as f64 / 4.0);
            let tree = runtime.accessibility_json().to_string();
            assert!(
                tree.contains(copy),
                "missing reply for {notice:?} {conversation:?}"
            );
            assert!(!tree.contains("could not verify an answer"));
            assert!(runtime.find(REPLY).is_some());
            assert!(runtime.find(COMPOSER).is_some());
            assert!(runtime.find(SOURCES_TOGGLE).is_none());
        }
    }

    #[test]
    fn queued_turn_rises_once_and_keeps_its_slot_on_acceptance() {
        const TURN: &str = "inbox-qa-turn-fixture-queued";
        for reduced_motion in [false, true] {
            let mut runtime = active_drawer((1280.0, 800.0));
            runtime.update(move |state, _| {
                state
                    .accessibility
                    .fixture_override
                    .as_mut()
                    .unwrap()
                    .reduced_motion = reduced_motion;
            });
            runtime.frame(0.5);
            runtime.update(|state, tx| {
                let context = MutationContext::new(state.registration.context.as_ref().unwrap());
                let request = QaTurnRequest::new(
                    "fixture-queued".into(),
                    context,
                    "What is the current memory usage?".into(),
                    None,
                )
                .unwrap();
                state.inbox_qa.pending = Some(Pending { request });
                tx.scroll_to_bottom_instant(crate::inbox::SCROLL);
            });
            runtime.frame(0.0);
            let (exchange, _) = runtime.presented("inbox-qa-exchange").unwrap();
            let (queued, _) = runtime.presented(TURN).unwrap();
            if reduced_motion {
                assert_eq!(queued, exchange);
            } else {
                assert!(queued[1] > exchange[1] + 1.0);
            }
            assert_eq!(runtime.scroll_at_bottom(crate::inbox::SCROLL), Some(true));
            runtime.frame(0.5);
            runtime.update(|state, _| {
                let mut accepted = turn(QaTurnState::Running, 1);
                accepted.turn_id = "fixture-accepted".into();
                accepted.request_id = "fixture-queued".into();
                assert!(state.inbox_qa.accept_pending(&accepted));
                state.inbox_qa.adopt(Some(accepted));
            });
            runtime.frame(0.0);
            let (exchange, _) = runtime.presented("inbox-qa-exchange").unwrap();
            let (accepted, _) = runtime.presented(TURN).unwrap();
            assert_eq!(accepted, exchange);
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
            (
                QaAvailabilityReason::InsufficientResources,
                "Not enough resources for Assistant",
                "Free memory or disk space",
            ),
            (
                QaAvailabilityReason::ModelProvisionFailed,
                "Assistant model provisioning failed",
                "Check network access and free disk space",
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
    fn embedded_startup_stages_appear_in_the_inbox_footer_and_connection_settings() {
        let stages = [
            (
                QaPhase::Starting,
                QaStartupStage::DownloadingPack,
                "Downloading QA pack",
            ),
            (
                QaPhase::Unavailable,
                QaStartupStage::VerifyingPack,
                "Verifying QA pack",
            ),
            (
                QaPhase::Starting,
                QaStartupStage::LoadingModel,
                "Loading QA model",
            ),
            (
                QaPhase::Starting,
                QaStartupStage::Activating,
                "Activating QA model",
            ),
        ];
        let mut runtime = active_drawer((640.0, 480.0));
        for (index, (phase, stage, label)) in stages.into_iter().enumerate() {
            let now = 1.0 + index as f64;
            runtime.update(|state, _| {
                state.backend_kind = BackendKind::Local;
                let mut settings = QaSettings::new(true, phase, 2, 2, true);
                settings.startup_stage = Some(stage);
                if phase == QaPhase::Unavailable {
                    settings.availability_reason = Some(QaAvailabilityReason::PackMissing);
                }
                state.inbox_qa.apply_settings(settings, now);
            });
            runtime.frame(now);
            let tree = runtime.accessibility_json();
            assert_eq!(
                accessibility_nodes_with_key(&tree, "inbox-qa-startup-status"),
                1
            );
            assert!(tree.to_string().contains(label));
            assert!(runtime.find(DETAILS_TOGGLE).is_none());
            assert!(runtime.find(COMPOSER).is_none());
        }

        runtime.update(|state, _| {
            state.inbox.close();
            state.open_connection_settings();
        });
        runtime.frame(5.0);
        runtime.frame(5.25);
        let tree = runtime.accessibility_json();
        assert_eq!(
            accessibility_nodes_with_key(&tree, "inbox-qa-settings-status"),
            1
        );
        assert!(tree.to_string().contains("Activating QA model"));
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
        state.backend_kind = BackendKind::Local;
        state.connection.origin = TextEdit::new("local://embedded");
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
