//! Scoped Assistant shortcuts for Jobs and Inbox.

use fleetworks_contracts::chat::ChatSourceKind;
use fleetworks_contracts::chat::ChatSourceReference;
use fleetworks_contracts::context::Permission;
use rune::El;
use rune_kit::ControlSize;
use rune_kit::IconButton;
use rune_kit::IconButtonVariant;
use rune_kit::Kit;

use crate::app::Load;
use crate::app::Page;
use crate::icons;
use crate::inbox_qa::SourceOrigin;
use crate::logs::ActivityTab;
use crate::State;

pub(crate) const JOB_EXPLAIN: &str = "triage-explain";

impl State {
    fn can_explain_activity(&self) -> bool {
        self.supports_workstation_triage()
            && self.assistant_settings_visible()
            && self.permission(Permission::SubmitInteractive)
    }

    fn permits_triage_source(&self, source: &ChatSourceReference) -> bool {
        match source.kind {
            ChatSourceKind::Job => self.permission(Permission::ReadWorkstationActivity),
            ChatSourceKind::Readiness => self.permission(Permission::ReadBackend),
            _ => false,
        }
    }

    fn triage_job_source(&self) -> Option<ChatSourceReference> {
        if !self.can_explain_activity()
            || self.page != Page::Jobs
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
        if !self.can_explain_activity() {
            return None;
        }
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
}

pub(crate) fn explain_notice_button(
    state: &State,
    notice_id: u64,
    title: &str,
    kit: &Kit,
) -> Option<El> {
    let source = state.explain_notice_source(notice_id)?;
    let title = title.to_owned();
    Some(
        explain_icon(state, "Explain this notification in Assistant")
            .on_press_tx(move |state: &mut State, tx| {
                if state.explain_notice_source(notice_id).as_ref() == Some(&source) {
                    state.prefill_inbox_question(&source, &title, SourceOrigin::Notification, tx);
                }
            })
            .build(kit)
            .key(format!("inbox-item-{notice_id}-explain")),
    )
}

pub(crate) fn explain_job_button(state: &State, kit: &Kit) -> Option<El> {
    let source = state.triage_job_source()?;
    Some(
        explain_icon(state, "Explain this job in Assistant")
            .on_press_tx(move |state: &mut State, tx| {
                let Load::Ready(detail) = &state.backend.detail else {
                    return;
                };
                if state.triage_job_source().as_ref() == Some(&source) {
                    let title = detail.job.title.clone();
                    state.prefill_inbox_question(&source, &title, SourceOrigin::Selection, tx);
                }
            })
            .build(kit)
            .key(JOB_EXPLAIN),
    )
}

fn explain_icon(state: &State, help: &str) -> IconButton {
    IconButton::new(&icons::ASSISTANT_EXPLAIN)
        .variant(IconButtonVariant::Ghost)
        .control_size(ControlSize::Large)
        .label("Explain in Assistant")
        .help(help)
        .disabled(!state.can_prefill_inbox_question())
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
        assert!(tree.contains("Attached evidence: Readiness · Actor search was unavailable"));
        assert!(tree.contains("Remove readiness source: Readiness · Actor search was unavailable"));
        assert!(runtime.find("inbox-qa-attached-source").is_some());
        assert!(runtime.find("inbox-item-1-detail").is_none());
        assert!(runtime.find("inbox-qa-exchange").is_none());
        assert_eq!(runtime.state.inbox.unread(), unread);

        press(&mut runtime, "inbox-qa-clear-source", 1.25);
        assert!(runtime.find("inbox-qa-attached-source").is_none());
        assert!(runtime
            .accessibility_json()
            .to_string()
            .contains("What happened with this readiness check?"));
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
    fn explain_private_job_prefills_the_composer_without_submitting() {
        let mut initial = private_job_state();
        initial.seed_inbox_qa_fixture(QaSettings::new(true, QaPhase::Ready, 2, 1, true), None);
        let mut runtime = runtime(initial);
        let buttons: Vec<_> = runtime
            .elements()
            .into_iter()
            .filter(|element| {
                !element.exiting && element.label.as_deref() == Some("Explain in Assistant")
            })
            .collect();
        assert_eq!(buttons.len(), 1);
        let button = &buttons[0];
        assert_eq!(button.key.as_deref(), Some(JOB_EXPLAIN));
        assert_eq!(button.role, "button");
        assert!(button.text.is_none());
        assert!((button.rect[2] - button.rect[3]).abs() < 0.5);

        press(&mut runtime, JOB_EXPLAIN, 0.5);
        let tree = runtime.accessibility_json().to_string();
        assert!(tree.contains("What happened with this job?"));
        assert!(tree.contains("Attached evidence: Job · Actor index"));
        assert!(runtime.find("inbox-qa-exchange").is_none());

        runtime.key(KeyEvent {
            key: Key::Escape,
            mods: Mods::default(),
        });
        runtime.frame(0.9);
        assert!(!runtime.state.inbox.open);
        assert!(runtime.elements().iter().any(|element| {
            element.key.as_deref() == Some(JOB_EXPLAIN) && element.focused && !element.exiting
        }));
    }

    #[test]
    fn shared_job_hides_explanation() {
        let mut runtime = runtime(private_job_state());
        assert!(runtime.find(JOB_EXPLAIN).is_some());
        runtime.update(|state, _| {
            if let Load::Ready(detail) = &mut state.backend.detail {
                detail.job.status_only = true;
                detail.private_details_available = false;
            }
        });
        runtime.frame(0.8);
        assert!(runtime.find(JOB_EXPLAIN).is_none());
        assert!(runtime
            .accessibility_json()
            .to_string()
            .contains("Private details and logs are unavailable for this shared job."));
    }
}
