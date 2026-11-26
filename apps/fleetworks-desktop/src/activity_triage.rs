//! Read-only Jobs triage and scoped Assistant shortcuts.

use fleetworks_contracts::chat::ChatSourceKind;
use fleetworks_contracts::chat::ChatSourceReference;
use fleetworks_contracts::chat::QaRelatedActivity;
use fleetworks_contracts::chat::QaRelatedActivityRequest;
use fleetworks_contracts::chat::QaRelationshipReason;
use fleetworks_contracts::context::Permission;
use rune::El;
use rune::LiveRegion;
use rune::Tx;
use rune_kit::ControlSize;
use rune_kit::IconButton;
use rune_kit::IconButtonVariant;
use rune_kit::Kit;

use crate::accessibility::ViewPreferences;
use crate::app::Load;
use crate::app::Page;
use crate::connection::RequestTask;
use crate::icons;
use crate::logs::ActivityTab;
use crate::material::MetalFinish;
use crate::ui_components::ActionButton;
use crate::views::quiet;
use crate::State;

#[derive(Default)]
pub(crate) struct ActivityTriage {
    revision: u64,
    lookup: Option<Lookup>,
}

struct Lookup {
    source: ChatSourceReference,
    epoch: u64,
    origin_revision: u64,
    generation: u64,
    outcome: Outcome,
}

enum Outcome {
    Pending { _task: Option<RequestTask> },
    Settled(QaRelatedActivity),
}

pub(crate) enum RelatedLocation {
    Job,
    Assistant,
}

impl Lookup {
    fn visible(&self, state: &State) -> bool {
        self.epoch == state.connection.epoch
            && self.origin_revision == state.origin_revision
            && state.triage_generation() == Some(self.generation)
            && state.triage_job_source().as_ref() == Some(&self.source)
    }
}

impl State {
    fn permits_triage_source(&self, source: &ChatSourceReference) -> bool {
        match source.kind {
            ChatSourceKind::Job => self.permission(Permission::ReadWorkstationActivity),
            ChatSourceKind::Readiness => self.permission(Permission::ReadBackend),
            _ => false,
        }
    }

    fn triage_job_source(&self) -> Option<ChatSourceReference> {
        self.triage_generation()?;
        if self.page != Page::Jobs
            || self.activity_tab != ActivityTab::Jobs
            || self.inbox.open
            || self.settings_open
            || self.setup_gateway
        {
            return None;
        }
        let Load::Ready(detail) = &self.backend.detail else {
            return None;
        };
        if !detail.private_details_available
            || !detail.job.private_details_available
            || detail.job.status_only
            || self.triage_job_is_shared(&detail.job.id)
            || self.backend.selected_job.as_deref() != Some(detail.job.id.as_str())
        {
            return None;
        }
        let source = ChatSourceReference::new(ChatSourceKind::Job, detail.job.id.clone()).ok()?;
        self.permits_triage_source(&source).then_some(source)
    }

    fn explain_notice_source(&self, notice_id: u64) -> Option<ChatSourceReference> {
        self.triage_generation()?;
        let source = self.notification_source(notice_id)?;
        self.permits_triage_source(&source).then_some(source)
    }

    pub(crate) fn triage_job_is_shared(&self, id: &str) -> bool {
        matches!(&self.backend.detail, Load::Ready(detail) if detail.job.id == id
            && (!detail.private_details_available
                || !detail.job.private_details_available
                || detail.job.status_only))
            || self
                .scoped_jobs
                .page
                .as_ref()
                .and_then(|page| page.rows.iter().find(|job| job.id == id))
                .is_some_and(|job| !job.private_details_available || job.status_only)
    }

    pub(crate) fn cancel_activity_triage(&mut self) {
        self.activity_triage.lookup = None;
    }

    pub(crate) fn poll_activity_triage(&mut self) -> bool {
        if self
            .activity_triage
            .lookup
            .as_ref()
            .is_some_and(|lookup| !lookup.visible(self))
        {
            self.cancel_activity_triage();
            return true;
        }
        false
    }

    fn find_related_activity(&mut self, source: &ChatSourceReference) {
        let Some(generation) = self.triage_generation() else {
            return;
        };
        if self.triage_job_source().as_ref() != Some(source) {
            return;
        }
        if self.activity_triage.lookup.as_ref().is_some_and(|lookup| {
            lookup.visible(self) && matches!(lookup.outcome, Outcome::Pending { .. })
        }) {
            return;
        }
        let Ok(request) = QaRelatedActivityRequest::new(source.clone()) else {
            return;
        };
        self.activity_triage.revision = self.activity_triage.revision.wrapping_add(1);
        let revision = self.activity_triage.revision;
        let outcome = if self.fixture {
            Outcome::Pending { _task: None }
        } else if let (Some(services), Some(client), Some(context)) =
            (&self.services, &self.client, &self.registration.context)
        {
            let task = client
                .is_local_ipc()
                .then(|| client.with_registered_context(context).ok())
                .flatten()
                .zip(services.ui.get().cloned())
                .map(|(client, ui)| {
                    let task = services.runtime.spawn(async move {
                        let result = client
                            .qa_related_activity(&request)
                            .await
                            .unwrap_or(QaRelatedActivity::TriageUnavailable);
                        ui.update(move |state, _| state.complete_activity_triage(revision, result));
                    });
                    RequestTask(task.abort_handle())
                });
            match task {
                Some(task) => Outcome::Pending { _task: Some(task) },
                None => Outcome::Settled(QaRelatedActivity::TriageUnavailable),
            }
        } else {
            Outcome::Settled(QaRelatedActivity::TriageUnavailable)
        };
        self.activity_triage.lookup = Some(Lookup {
            source: source.clone(),
            epoch: self.connection.epoch,
            origin_revision: self.origin_revision,
            generation,
            outcome,
        });
    }

    fn complete_activity_triage(&mut self, revision: u64, result: QaRelatedActivity) {
        if revision != self.activity_triage.revision {
            return;
        }
        // Authorization or selection can change before the next shell poll.
        self.poll_activity_triage();
        if let Some(lookup) = &mut self.activity_triage.lookup {
            if matches!(lookup.outcome, Outcome::Pending { .. }) {
                lookup.outcome = Outcome::Settled(result);
            }
        }
    }

    fn open_related_activity(&mut self, target: &ChatSourceReference, tx: &mut Tx) {
        if !self.supports_workstation_triage()
            || !self.assistant_settings_visible()
            || !self.permits_triage_source(target)
        {
            return;
        }
        if self.inbox.open {
            self.close_inbox(tx);
        }
        match target.kind {
            ChatSourceKind::Job => self.open_job(&target.id),
            ChatSourceKind::Readiness => {
                let id = match &self.readiness {
                    Load::Ready(readiness) => readiness
                        .checks
                        .iter()
                        .map(|check| check.id)
                        .find(|id| id.as_str() == target.id),
                    _ => None,
                };
                if let Some(id) = id {
                    self.open_readiness_notice(id, tx);
                } else {
                    self.navigate(Page::Workstation);
                    self.workstation_status_checks_open = true;
                }
            }
            _ => {}
        }
    }

    /// Accept a synthetic related result on the production Jobs view.
    #[doc(hidden)]
    pub fn seed_activity_triage_fixture(&mut self, result: QaRelatedActivity) {
        if !self.fixture {
            return;
        }
        if let Some(source) = self.triage_job_source() {
            self.find_related_activity(&source);
            self.complete_activity_triage(self.activity_triage.revision, result);
        }
    }
}

pub(crate) fn explain_notice_button(state: &State, notice_id: u64, kit: &Kit) -> Option<El> {
    if !state.permission(Permission::SubmitInteractive) {
        return None;
    }
    let source = state.explain_notice_source(notice_id)?;
    Some(
        IconButton::new(&icons::ASSISTANT)
            .variant(IconButtonVariant::Ghost)
            .control_size(ControlSize::Large)
            .label("Explain in Assistant")
            .help("Explain this notification in Assistant")
            .disabled(!state.can_prefill_inbox_question())
            .on_press_tx(move |state: &mut State, tx| {
                if state.explain_notice_source(notice_id).as_ref() == Some(&source) {
                    state.prefill_inbox_question(&source, tx);
                }
            })
            .build(kit)
            .key(format!("inbox-item-{notice_id}-explain")),
    )
}

pub(crate) fn job_actions(state: &State, kit: &Kit, preferences: ViewPreferences) -> Option<El> {
    let source = state.triage_job_source()?;
    let lookup = state
        .activity_triage
        .lookup
        .as_ref()
        .filter(|lookup| lookup.visible(state));
    let pending = lookup.is_some_and(|lookup| matches!(lookup.outcome, Outcome::Pending { .. }));
    let lookup_source = source.clone();
    let mut actions = rune::row().w_full().gap(6.0).flex_wrap().child(
        ActionButton::new("Find related activity")
            .key("triage-find-related")
            .finish(MetalFinish::Gunmetal)
            .busy(pending)
            .disabled(pending)
            .on_press(move |state: &mut State| state.find_related_activity(&lookup_source))
            .build(kit, preferences, None),
    );
    if state.permission(Permission::SubmitInteractive) {
        actions = actions.child(
            ActionButton::new("Explain in Assistant")
                .key("triage-explain")
                .finish(MetalFinish::Gunmetal)
                .disabled(!state.can_prefill_inbox_question())
                .on_press_tx(move |state: &mut State, tx| {
                    if state.triage_job_source().as_ref() == Some(&source) {
                        state.prefill_inbox_question(&source, tx);
                    }
                })
                .build(kit, preferences, None),
        );
    }
    let result = lookup.map(|lookup| match &lookup.outcome {
        Outcome::Pending { .. } => triage_status("Finding related activity…", kit),
        Outcome::Settled(result) => {
            result_view(state, result, RelatedLocation::Job, kit, preferences)
        }
    });
    Some(
        rune::col()
            .key("activity-triage")
            .w_full()
            .gap(4.0)
            .child(actions)
            .children(result),
    )
}

pub(crate) fn result_view(
    state: &State,
    result: &QaRelatedActivity,
    location: RelatedLocation,
    kit: &Kit,
    preferences: ViewPreferences,
) -> El {
    match result {
        QaRelatedActivity::Found { target, reason } if state.permits_triage_source(target) => {
            let label = match (reason, target.kind) {
                (QaRelationshipReason::CurrentStatus, ChatSourceKind::Readiness) => {
                    "Current status"
                }
                (_, ChatSourceKind::Job) => "Open related job",
                (_, ChatSourceKind::Readiness) => "View related status",
                _ => return triage_status("No related activity found", kit),
            };
            let detail = match reason {
                QaRelationshipReason::CurrentStatus => "Latest readiness check",
                QaRelationshipReason::StageCheck => "Related readiness check",
                QaRelationshipReason::SharedOperation => "Same operation family",
                _ => "Related activity",
            };
            let target = target.clone();
            let reason = *reason;
            let key = match location {
                RelatedLocation::Job => "triage-open-related",
                RelatedLocation::Assistant => "inbox-qa-open-related",
            };
            rune::row()
                .key("triage-result")
                .w_full()
                .gap(6.0)
                .flex_wrap()
                .center_y()
                .child(
                    quiet(detail, kit)
                        .role("status")
                        .live_region(LiveRegion::Polite),
                )
                .child(
                    ActionButton::new(label)
                        .key(key)
                        .finish(MetalFinish::Gunmetal)
                        .on_press_tx(move |state: &mut State, tx| {
                            let active = match location {
                                RelatedLocation::Job => state.activity_triage.lookup.as_ref().is_some_and(|lookup| {
                                    lookup.visible(state)
                                        && matches!(&lookup.outcome, Outcome::Settled(
                                            QaRelatedActivity::Found { target: current, reason: current_reason }
                                        ) if current == &target && *current_reason == reason)
                                }),
                                RelatedLocation::Assistant => matches!(state.canonical_related_activity(),
                                    Some(QaRelatedActivity::Found { target: current, reason: current_reason })
                                        if current == &target && *current_reason == reason
                                ),
                            };
                            if active {
                                state.open_related_activity(&target, tx);
                            }
                        })
                        .build(kit, preferences, None),
                )
        }
        QaRelatedActivity::NoRelatedActivity => triage_status("No related activity found", kit),
        QaRelatedActivity::Clarify => triage_status("Specify the activity in Assistant", kit),
        _ => triage_status("Triage unavailable", kit),
    }
}

fn triage_status(text: &str, kit: &Kit) -> El {
    quiet(text, kit)
        .key("triage-result")
        .role("status")
        .live_region(LiveRegion::Polite)
}

#[cfg(test)]
mod tests {
    use fleetworks_contracts::chat::QaPhase;
    use fleetworks_contracts::chat::QaSettings;
    use fleetworks_contracts::chat::QA_TRIAGE_FEATURE;
    use fleetworks_workstation::native::BackendKind;
    use rune::Env;
    use rune::Key;
    use rune::KeyEvent;
    use rune::Mods;
    use rune::Runtime;
    use rune::TextEdit;

    use super::*;

    fn triage_state() -> State {
        let mut state = crate::tests::registered_fixture();
        state.backend_kind = BackendKind::Local;
        state.connection.origin = TextEdit::new("local://embedded");
        let capabilities = state
            .connection
            .capabilities
            .as_mut()
            .expect("capabilities");
        capabilities.features.push(QA_TRIAGE_FEATURE.into());
        capabilities.permissions.push(Permission::SubmitInteractive);
        state.seed_inbox_qa_fixture(QaSettings::new(true, QaPhase::Starting, 1, 1, true), None);
        state.page = Page::Workstation;
        state
    }

    fn runtime(initial: State) -> Runtime<State> {
        let mut runtime = Runtime::new(
            initial,
            crate::app::view,
            (640.0, 480.0),
            Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        runtime.reduced_motion = true;
        runtime.frame(0.0);
        runtime.frame(0.25);
        runtime
    }

    fn press(runtime: &mut Runtime<State>, key: &'static str, now: f64) {
        runtime.update(|_, tx| {
            tx.scroll_into_view(key);
            tx.focus(key, true);
        });
        runtime.frame(now);
        assert!(runtime.elements().iter().any(|element| {
            element.key.as_deref() == Some(key)
                && element.focused
                && element.visible == element.rect
        }));
        runtime.key(KeyEvent {
            key: Key::Text(" ".into()),
            mods: Mods::default(),
        });
        runtime.frame(now + 0.1);
    }

    fn readiness_result(reason: QaRelationshipReason) -> QaRelatedActivity {
        QaRelatedActivity::Found {
            target: ChatSourceReference::new(ChatSourceKind::Readiness, "actor_search".into())
                .expect("typed fixture source"),
            reason,
        }
    }

    #[test]
    fn notice_icon_prefills_from_the_row_once_the_generator_is_ready() {
        const EXPLAIN: &str = "inbox-item-1-explain";
        let mut initial = triage_state();
        initial.fixture_show_triage_notice();
        let mut runtime = runtime(initial);
        let tree = runtime.accessibility_json().to_string();
        assert!(tree.contains("View status"));
        assert!(tree.contains("Preparing the selected Assistant generator"));
        assert!(runtime.find("inbox-qa-composer").is_none());
        assert!(runtime
            .elements()
            .iter()
            .any(|element| { element.key.as_deref() == Some(EXPLAIN) && !element.focusable }));
        assert!(runtime.find("triage-find-related").is_none());
        press(&mut runtime, "inbox-item-1-row", 0.5);
        assert!(runtime.find("inbox-item-1-detail").is_none());
        let unread = runtime.state.inbox.unread();

        runtime.update(|state, _| {
            state.seed_inbox_qa_fixture(QaSettings::new(true, QaPhase::Ready, 2, 1, true), None);
        });
        runtime.frame(0.9);
        press(&mut runtime, EXPLAIN, 1.0);
        let tree = runtime.accessibility_json().to_string();
        assert!(tree.contains("What happened with this readiness check?"));
        assert!(tree.contains("Remove readiness source"));
        assert!(runtime.find("inbox-item-1-detail").is_none());
        assert!(runtime.find("inbox-qa-exchange").is_none());
        assert_eq!(runtime.state.inbox.unread(), unread);
    }

    fn private_job_state() -> State {
        let mut initial = triage_state();
        initial.page = Page::Jobs;
        let job = crate::backend::ActivityJob {
            id: "private-index-job".into(),
            app: "Workstation".into(),
            title: "Actor index".into(),
            operation: "actor_reindex".into(),
            state: "running".into(),
            phase: "indexing".into(),
            progress_done: None,
            progress_total: None,
            error_summary: String::new(),
            private_details_available: true,
            activity_actions_available: false,
            status_only: false,
            can_cancel: false,
            can_retry: false,
            action_reason_code: String::new(),
        };
        initial.backend.selected_job = Some(job.id.clone());
        initial.backend.detail = Load::Ready(crate::backend::JobDetail {
            job,
            stages: Vec::new(),
            private_details_available: true,
            activity_actions_available: false,
            maintenance_result: None,
        });
        initial
    }

    #[test]
    fn selected_job_becoming_status_only_discards_its_pending_result() {
        let mut runtime = runtime(private_job_state());
        press(&mut runtime, "triage-find-related", 0.5);
        let revision = runtime.state.activity_triage.revision;
        runtime.update(move |state, _| {
            if let Load::Ready(detail) = &mut state.backend.detail {
                detail.job.status_only = true;
                detail.private_details_available = false;
            }
            state.complete_activity_triage(
                revision,
                readiness_result(QaRelationshipReason::StageCheck),
            );
        });
        runtime.frame(0.8);
        assert!(runtime.find("triage-open-related").is_none());
        assert!(runtime.find("triage-find-related").is_none());
        assert!(runtime
            .accessibility_json()
            .to_string()
            .contains("Private details and logs are unavailable for this shared job."));
        assert!(runtime.state.activity_triage.lookup.is_none());
    }

    #[test]
    fn superseded_job_response_never_replaces_newer_inline_state() {
        let mut runtime = runtime(private_job_state());
        press(&mut runtime, "triage-find-related", 0.5);
        let obsolete = runtime.state.activity_triage.revision;
        runtime.update(|state, _| {
            let job_id = "new-index-job".to_owned();
            if let Load::Ready(detail) = &mut state.backend.detail {
                detail.job.id = job_id.clone();
            }
            state.backend.selected_job = Some(job_id);
        });
        runtime.frame(0.8);
        press(&mut runtime, "triage-find-related", 1.0);
        let current = runtime.state.activity_triage.revision;
        assert!(current > obsolete);
        runtime.update(move |state, _| {
            state.complete_activity_triage(current, QaRelatedActivity::NoRelatedActivity);
            state.complete_activity_triage(
                obsolete,
                readiness_result(QaRelationshipReason::CurrentStatus),
            );
        });
        runtime.frame(1.3);
        let tree = runtime.accessibility_json().to_string();
        assert!(tree.contains("No related activity found"));
        assert!(!tree.contains("Current status"));
        assert!(runtime.find("triage-open-related").is_none());

        runtime.update(|state, _| {
            state.seed_inbox_qa_fixture(QaSettings::new(false, QaPhase::Off, 2, 2, true), None);
        });
        runtime.frame(1.5);
        assert!(runtime.find("triage-find-related").is_none());
        assert!(runtime.find("inbox-toggle").is_some());
    }
}
