//! The cold-start workflow uses the same registered maintenance operations as Workstation Status.

use std::path::PathBuf;

use fleetworks_contracts::context::Permission;
use fleetworks_contracts::readiness::DatasetKind;
use fleetworks_contracts::readiness::DatasetRef;
use fleetworks_contracts::readiness::MaintenanceOperation;
use fleetworks_contracts::readiness::ReadinessCheck;
use fleetworks_contracts::readiness::ReadinessCheckId;
use fleetworks_contracts::readiness::ReadinessState;
use fleetworks_contracts::readiness::WorkstationReadiness;
use fleetworks_contracts::DatasetUpload;
use rune::El;
use rune::TextEdit;
use rune::Tx;
use rune_kit::Kit;
use rune_kit::SegmentedControl;
use rune_kit::TextInput;

use crate::accessibility::ViewPreferences;
use crate::app::Load;
use crate::backend;
use crate::backend_views;
use crate::maintenance::OperationJobRecord;
use crate::material::MetalFinish;
use crate::startup;
use crate::startup_checks;
use crate::startup_checks::CheckStatus;
use crate::theme::StatusTone;
use crate::ui_components;
use crate::ui_components::ActionButton;
use crate::views;
use crate::State;

pub(crate) const PRIMARY: &str = "startup-data-primary";
const INFERENCE_DOT: &str = "startup-data-step-inference";
const CHECKS_DOT: &str = "startup-data-step-checks";

#[derive(Clone, Copy, Eq, PartialEq)]
enum CsvSource {
    Local,
    Bundled,
    Github,
}

impl CsvSource {
    fn label(self) -> &'static str {
        match self {
            Self::Local => "Local CSV",
            Self::Bundled => "Bundled CSV",
            Self::Github => "GitHub CSV URL",
        }
    }
}

enum Bundle {
    Unchecked,
    Missing,
    Available { path: PathBuf, attempted: bool },
}

struct CsvInput {
    source: CsvSource,
    bundle: Bundle,
    url: TextEdit,
}

impl CsvInput {
    fn new(kind: DatasetKind) -> Self {
        Self {
            source: CsvSource::Local,
            bundle: Bundle::Unchecked,
            url: TextEdit::new(crate::github_dataset::default_url(kind).unwrap_or_default()),
        }
    }
}

pub(crate) struct CsvInputs {
    actors: CsvInput,
}

impl Default for CsvInputs {
    fn default() -> Self {
        Self {
            actors: CsvInput::new(DatasetKind::ActorCatalog),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Step {
    Seed,
    Index,
}

/// Keep a skipped setup slide out of this visit's sequence after active work settles.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum DataSlides {
    #[default]
    All,
    IndexOnly,
}

impl DataSlides {
    fn for_observed_work(state: &State) -> Self {
        if operation_active(state, Step::Index) || operation_active(state, Step::Seed) {
            Self::IndexOnly
        } else {
            Self::All
        }
    }

    fn includes(self, step: Step) -> bool {
        self == Self::All || step == Step::Index
    }
}

impl Step {
    const ALL: [Self; 2] = [Self::Seed, Self::Index];

    fn number(self) -> usize {
        match self {
            Self::Seed => 1,
            Self::Index => 2,
        }
    }

    fn key(self) -> &'static str {
        match self {
            Self::Seed => "startup-data-step-seed",
            Self::Index => "startup-data-step-index",
        }
    }

    fn review_key(self) -> &'static str {
        match self {
            Self::Seed => "startup-data-review-seed",
            Self::Index => "startup-data-review-index",
        }
    }

    fn reset_key(self) -> &'static str {
        match self {
            Self::Seed => "startup-data-reset-seed",
            Self::Index => "startup-data-reset-index",
        }
    }

    fn short_name(self) -> &'static str {
        match self {
            Self::Seed => "Actor CSV",
            Self::Index => "Search index",
        }
    }

    fn reset_label(self) -> &'static str {
        match self {
            Self::Seed => "Reset actor CSV",
            Self::Index => "Reset index input",
        }
    }

    pub(crate) fn dataset_kind(self) -> Option<DatasetKind> {
        match self {
            Self::Seed => Some(DatasetKind::ActorCatalog),
            Self::Index => None,
        }
    }

    fn csv_input(self, inputs: &CsvInputs) -> Option<&CsvInput> {
        match self {
            Self::Seed => Some(&inputs.actors),
            Self::Index => None,
        }
    }

    fn csv_input_mut(self, inputs: &mut CsvInputs) -> Option<&mut CsvInput> {
        match self {
            Self::Seed => Some(&mut inputs.actors),
            Self::Index => None,
        }
    }

    fn reset(self, state: &State) -> bool {
        state.startup.data_reset[self.number() - 1]
    }

    fn title(self) -> &'static str {
        match self {
            Self::Seed => "Seed the actor catalog",
            Self::Index => "Build the actor search index",
        }
    }

    fn operation(self) -> MaintenanceOperation {
        match self {
            Self::Seed => MaintenanceOperation::SeedActors,
            Self::Index => MaintenanceOperation::ReindexActors,
        }
    }

    fn check(self) -> ReadinessCheckId {
        match self {
            Self::Seed => ReadinessCheckId::ActorCatalog,
            Self::Index => ReadinessCheckId::ActorSearch,
        }
    }

    fn job_operation(self) -> &'static str {
        match self {
            Self::Seed => "actor_seed",
            Self::Index => "actor_reindex",
        }
    }

    fn action(self) -> &'static str {
        match self {
            Self::Seed if cfg!(target_os = "linux") => "Enter actor CSV path",
            Self::Seed => "Choose actor CSV…",
            Self::Index => "Build actor index",
        }
    }

    fn validated_action(self) -> &'static str {
        match self {
            Self::Seed => "Seed actors",
            Self::Index => self.action(),
        }
    }

    fn start(self, state: &mut State, tx: &mut Tx) {
        if state.startup.data_step != Some(self)
            || !matches!(stage(state, self), Stage::Action | Stage::Failed(_))
            || state.maintenance.busy()
        {
            return;
        }
        state.maintenance.message = None;
        if self == Self::Index {
            if self.reset(state) {
                if !state.readiness_pending()
                    && current_check(state, Step::Seed)
                        .is_some_and(|check| check.state == ReadinessState::Ready)
                {
                    state.startup.data_reset[self.number() - 1] = false;
                    focus_stage(tx);
                }
                return;
            }
            self.preview(state, tx);
        } else if validated_csv(state, self).is_some() {
            self.preview(state, tx);
        } else if let Some((kind, input)) = self
            .dataset_kind()
            .zip(self.csv_input(&state.startup.csv_inputs))
        {
            match input.source {
                CsvSource::Bundled => {
                    state.validate_bundled_startup_csv(self, kind);
                }
                CsvSource::Github => {
                    state.download_github_dataset(kind, input.url.text.clone());
                }
                CsvSource::Local if cfg!(target_os = "linux") => state.enter_csv_path(kind, tx),
                CsvSource::Local => state.choose_dataset(kind, tx),
            }
        }
    }

    fn use_saved_csv(self, state: &mut State, tx: &mut Tx) {
        if state.startup.data_step == Some(self)
            && matches!(stage(state, self), Stage::Action | Stage::Failed(_))
            && !state.maintenance.busy()
        {
            if let Some(dataset) = saved_dataset(state, self).cloned() {
                state.maintenance.selected_dataset = Some(dataset);
                self.preview(state, tx);
            }
        }
    }

    fn preview(self, state: &mut State, tx: &mut Tx) {
        state.preview_maintenance(self.operation());
        if state.maintenance.confirmation.is_some() {
            tx.scroll_into_view("maintenance-confirmation");
            tx.focus("maintenance-confirm", tx.by_keys());
        }
    }
}

#[derive(Clone, Copy)]
enum Stage<'a> {
    Observing,
    Action,
    AwaitingCatalog,
    Working(Option<&'a backend::JobDetail>),
    ObservedWork,
    Verifying,
    Failed(&'a backend::JobDetail),
    Complete,
}

pub(crate) fn scoped_readiness(state: &State) -> Option<&WorkstationReadiness> {
    let Load::Ready(readiness) = &state.readiness else {
        return None;
    };
    let context = state.registration.context.as_ref()?;
    let receipt = readiness.registration.as_ref()?;
    (state.connected()
        && readiness.deployment_id == context.deployment_id
        && receipt.registration_id == context.registration_id
        && receipt.revision == context.registration_revision)
        .then_some(readiness)
}

fn current_check(state: &State, step: Step) -> Option<&ReadinessCheck> {
    scoped_readiness(state)?
        .checks
        .iter()
        .find(|check| check.id == step.check() && state.readiness_check_fresh(check))
}

fn tracked_job_detail(record: &OperationJobRecord, step: Step) -> Option<&backend::JobDetail> {
    record.detail.as_ref().filter(|detail| {
        detail.job.id == record.job_id && detail.job.operation == step.job_operation()
    })
}

fn active_tracked_job(state: &State, step: Step) -> Option<&backend::JobDetail> {
    if !state.connected() || state.registration.context.is_none() {
        return None;
    }
    state
        .maintenance
        .operation_job(step.operation())
        .filter(|record| record.poll_error.is_none())
        .and_then(|record| tracked_job_detail(record, step))
        .filter(|detail| backend::job_is_active(&detail.job.state))
}

fn operation_active(state: &State, step: Step) -> bool {
    current_check(state, step).is_some_and(|check| check.state == ReadinessState::Running)
        || active_tracked_job(state, step).is_some()
}

fn saved_dataset(state: &State, step: Step) -> Option<&DatasetRef> {
    let kind = step.dataset_kind()?;
    if step.reset(state) {
        return None;
    }
    if let Some(dataset) = state
        .maintenance
        .selected_dataset
        .as_ref()
        .filter(|dataset| dataset.kind == kind && dataset.validate().is_ok())
    {
        return Some(dataset);
    }
    let revision = state.configuration_revision()?;
    scoped_readiness(state)?
        .configured_inputs
        .iter()
        .filter(|input| {
            input.operation == step.operation() && input.configuration_revision == revision
        })
        .filter_map(|input| input.dataset.as_ref())
        .find(|dataset| dataset.kind == kind && dataset.validate().is_ok())
}

fn validated_csv(state: &State, step: Step) -> Option<&DatasetUpload> {
    let Load::Ready(upload) = &state.maintenance.upload else {
        return None;
    };
    (Some(upload.dataset.kind) == step.dataset_kind()
        && state.maintenance.selected_dataset.as_ref() == Some(&upload.dataset)
        && upload.dataset.validate().is_ok())
    .then_some(upload)
}

fn committed_seed_progress(state: &State, detail: &backend::JobDetail) -> Option<String> {
    if !state.permission(Permission::ReadWorkstationActivity)
        || state.registration.context.is_none()
        || !detail.private_details_available
        || !detail.job.private_details_available
        || detail.job.status_only
        || detail.job.operation != Step::Seed.job_operation()
    {
        return None;
    }
    let upload = validated_csv(state, Step::Seed)?;
    let result = detail.maintenance_result.as_ref()?;
    let (processed, total) = (result.processed?, result.total?);
    (result.operation == Step::Seed.job_operation()
        && result.dataset_sha256.as_deref() == Some(upload.dataset.sha256.as_str())
        && result.database_seeded == Some(false)
        && total > 0
        && processed <= total
        && upload.row_count.is_none_or(|validated| validated == total))
    .then(|| {
        format!(
            "{} / {} actor rows committed",
            format_row_count(processed),
            format_row_count(total)
        )
    })
}

fn pressure_explanation(error_summary: &str) -> Option<&'static str> {
    match error_summary {
        "setup_worker_memory_pressure" => Some("The backend reached its memory limit."),
        "setup_worker_disk_pressure" => {
            Some("Free space in the backend workspace fell below its disk limit.")
        }
        "setup_worker_pressure_unavailable" => {
            Some("The backend could not measure memory use or free workspace space.")
        }
        _ => None,
    }
}

fn catalog_hash(state: &State) -> Option<&str> {
    current_check(state, Step::Seed)?
        .artifact_ids
        .iter()
        .filter_map(|artifact| artifact.strip_prefix("actor-catalog-sha256:"))
        .find(|hash| hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

fn stage(state: &State, step: Step) -> Stage<'_> {
    let observation = current_check(state, step);
    if step == Step::Index && state.maintenance.pending_reindex.is_some() {
        return Stage::Action;
    }
    if state.maintenance.active_operation == Some(step.operation())
        && matches!(state.maintenance.receipt, Load::Loading)
    {
        return Stage::Working(None);
    }
    if step == Step::Index && state.startup.data_slides == DataSlides::IndexOnly {
        if let Some(detail) = active_tracked_job(state, step) {
            return Stage::Working(Some(detail));
        }
        if observation.is_some_and(|check| check.state == ReadinessState::Running) {
            return Stage::ObservedWork;
        }
        if operation_active(state, Step::Seed) {
            return Stage::AwaitingCatalog;
        }
    }
    if step.reset(state) {
        return Stage::Action;
    }
    if matches!(&state.maintenance.receipt, Load::Ready(receipt)
        if receipt.operation == step.operation() && receipt.already_completed)
    {
        if observation.is_some_and(|check| check.state == ReadinessState::Ready) {
            return Stage::Complete;
        }
        return Stage::Verifying;
    }
    let record = state.maintenance.operation_job(step.operation());
    if let Some(record) = record {
        let Some(detail) = tracked_job_detail(record, step) else {
            return Stage::Working(None);
        };
        let job = &detail.job;
        if backend::job_is_active(&job.state) {
            return Stage::Working(Some(detail));
        }
        if job.state != "completed" {
            return Stage::Failed(detail);
        }
        if observation.is_none_or(|check| check.state != ReadinessState::Ready) {
            return Stage::Verifying;
        }
    }
    match observation {
        Some(check) if check.state == ReadinessState::Ready => Stage::Complete,
        Some(check) if check.state == ReadinessState::Running => Stage::ObservedWork,
        Some(_) => Stage::Action,
        None if state.readiness_pending()
            || matches!(state.readiness, Load::Idle | Load::Pending | Load::Loading) =>
        {
            Stage::Observing
        }
        None => Stage::Action,
    }
}

fn first_unfinished(state: &State) -> Option<Step> {
    Step::ALL
        .into_iter()
        .filter(|&step| state.startup.data_slides.includes(step))
        .find(|&step| !matches!(stage(state, step), Stage::Complete))
}

fn can_review(state: &State, step: Step) -> bool {
    state.startup.data_slides.includes(step)
        && (matches!(stage(state, step), Stage::Complete)
            || Step::ALL
                .into_iter()
                .take(step.number() - 1)
                .all(|earlier| matches!(stage(state, earlier), Stage::Complete)))
}

fn can_reset(state: &State, step: Step) -> bool {
    can_review(state, step)
        && !state.maintenance.busy()
        && !matches!(
            stage(state, step),
            Stage::Working(_) | Stage::AwaitingCatalog
        )
        && !operation_active(state, step)
        && (step != Step::Index || state.maintenance.pending_reindex.is_none())
}

fn focus_stage(tx: &mut Tx) {
    tx.scroll_into_view("startup-data-stage");
    tx.focus("startup-data-stage", tx.by_keys());
}

impl State {
    pub(crate) fn enter_startup_data(&mut self) {
        self.startup.data_slides = DataSlides::for_observed_work(self);
        self.select_unfinished_startup_step();
    }

    fn select_unfinished_startup_step(&mut self) {
        self.startup.data_step = first_unfinished(self);
        self.startup.data_checks = self.startup.data_step.is_none();
    }

    pub(crate) fn drive_startup_dataset(&mut self) -> bool {
        let Some(step) = self.startup.data_step else {
            return false;
        };
        let Some(kind) = step.dataset_kind() else {
            return false;
        };
        if !self.connected() || self.registration.context.is_none() {
            return false;
        }
        if self.startup.has_wiring_steps()
            && self.maintenance.active_operation != Some(step.operation())
            && self.maintenance.operation_job(step.operation()).is_none()
        {
            // The first registered snapshot may arrive after the startup health-check timeout.
            let slides = DataSlides::for_observed_work(self);
            if !slides.includes(step) {
                self.clear_startup_csv(step);
                self.startup.data_slides = slides;
                self.select_unfinished_startup_step();
                return true;
            }
        }
        let mut changed = false;
        if step
            .csv_input(&self.startup.csv_inputs)
            .is_some_and(|input| matches!(input.bundle, Bundle::Unchecked))
        {
            let path = self.bundled_dataset_path(kind);
            let input = step
                .csv_input_mut(&mut self.startup.csv_inputs)
                .expect("dataset step has a CSV input");
            input.bundle = if let Some(path) = path {
                input.source = CsvSource::Bundled;
                Bundle::Available {
                    path,
                    attempted: false,
                }
            } else {
                Bundle::Missing
            };
            changed = true;
        }
        let first_attempt = step
            .csv_input(&self.startup.csv_inputs)
            .is_some_and(|input| {
                input.source == CsvSource::Bundled
                    && matches!(
                        input.bundle,
                        Bundle::Available {
                            attempted: false,
                            ..
                        }
                    )
            });
        if first_attempt {
            changed |= self.validate_bundled_startup_csv(step, kind);
        }
        changed
    }

    fn can_validate_bundled_startup_csv(&self, step: Step) -> bool {
        matches!(stage(self, step), Stage::Action | Stage::Failed(_))
            && !self.maintenance.busy()
            && self.maintenance.csv_path_prompt.is_none()
            && self.maintenance.confirmation.is_none()
            && validated_csv(self, step).is_none()
            && self.maintenance_disabled_reason(step.operation()).is_none()
    }

    fn validate_bundled_startup_csv(&mut self, step: Step, kind: DatasetKind) -> bool {
        if !self.can_validate_bundled_startup_csv(step) {
            return false;
        }
        let Some(CsvInput {
            source: CsvSource::Bundled,
            bundle: Bundle::Available { path, attempted },
            ..
        }) = step.csv_input_mut(&mut self.startup.csv_inputs)
        else {
            return false;
        };
        *attempted = true;
        let path = path.clone();
        self.stage_bundled_dataset(kind, path);
        true
    }

    fn startup_csv_source(&mut self, step: Step, source: CsvSource) {
        if self.startup.data_step != Some(step) {
            return;
        }
        let Some(input) = step.csv_input_mut(&mut self.startup.csv_inputs) else {
            return;
        };
        if input.source == source {
            return;
        }
        input.source = source;
        self.clear_startup_csv(step);
        self.drive_startup_dataset();
    }

    fn clear_startup_csv(&mut self, step: Step) {
        let Some(kind) = step.dataset_kind() else {
            return;
        };
        if self.startup.data_step == Some(step) {
            self.maintenance.discard_csv_draft();
            self.maintenance.message = None;
        }
        if self
            .maintenance
            .selected_dataset
            .as_ref()
            .is_some_and(|dataset| dataset.kind == kind)
        {
            self.maintenance.selected_dataset = None;
        }
        if self
            .maintenance
            .local_dataset
            .as_ref()
            .is_some_and(|dataset| dataset.kind == kind)
        {
            self.maintenance.local_dataset = None;
        }
        if self
            .maintenance
            .confirmation
            .as_ref()
            .is_some_and(|confirmation| confirmation.operation == step.operation())
        {
            self.cancel_maintenance_confirmation();
        }
    }

    pub(crate) fn complete_startup_input(&mut self, operation: MaintenanceOperation) {
        if let Some(step) = Step::ALL
            .into_iter()
            .find(|step| step.operation() == operation)
        {
            self.startup.data_reset[step.number() - 1] = false;
        }
    }

    fn next_startup_data_step(&mut self, tx: &mut Tx) {
        let Some(step) = self.startup.data_step else {
            return;
        };
        if !matches!(stage(self, step), Stage::Complete) {
            return;
        }
        self.maintenance.message = None;
        self.select_unfinished_startup_step();
        self.drive_startup_dataset();
        focus_stage(tx);
    }

    fn review_startup_input(&mut self, step: Step, tx: &mut Tx) {
        if self.startup.data_setup_active() && can_review(self, step) {
            if let Some(current) = self.startup.data_step.filter(|&current| current != step) {
                self.clear_startup_csv(current);
            }
            self.startup.data_checks = false;
            self.startup.data_step = Some(step);
            self.drive_startup_dataset();
            focus_stage(tx);
        }
    }

    fn reset_startup_input(&mut self, step: Step, tx: &mut Tx) {
        if !self.startup.data_setup_active() || !can_reset(self, step) {
            return;
        }
        if let Some(current) = self.startup.data_step.filter(|&current| current != step) {
            self.clear_startup_csv(current);
        }
        if let Some(kind) = step.dataset_kind() {
            self.clear_startup_csv(step);
            if let Some(input) = step.csv_input_mut(&mut self.startup.csv_inputs) {
                *input = CsvInput::new(kind);
            }
        } else if !self.readiness_pending() {
            self.refresh_readiness();
        }
        self.maintenance.message = None;
        self.startup.data_reset[step.number() - 1] = true;
        self.startup.data_checks = false;
        self.startup.data_step = Some(step);
        self.drive_startup_dataset();
        focus_stage(tx);
    }

    fn finish_startup_data_later(&mut self, tx: &mut Tx) {
        if !self.connected() || self.registration.context.is_none() {
            return;
        }
        if let Some(step) = self
            .startup
            .data_step
            .filter(|step| step.dataset_kind().is_some())
        {
            self.clear_startup_csv(step);
        } else {
            self.maintenance.discard_csv_draft();
        }
        self.cancel_maintenance_confirmation();
        // Leave accepted operations and their registered receipts running on the backend.
        self.startup.data_step = None;
        self.startup.data_checks = true;
        focus_stage(tx);
    }

    fn finish_startup_checks(&mut self, tx: &mut Tx) {
        if self.startup.data_checks && self.connected() && self.registration.context.is_some() {
            self.finish_startup(tx);
        }
    }

    fn refresh_startup_data(&mut self) {
        if !self.readiness_pending() {
            self.refresh_readiness();
        }
        self.backend_read(backend::ReadRequest::Profile);
        self.poll_maintenance_receipt_jobs();
    }

    pub(crate) fn startup_data_needs_observation(&self) -> bool {
        self.startup.data_step.is_some_and(|step| {
            matches!(
                stage(self, step),
                Stage::Observing | Stage::AwaitingCatalog | Stage::ObservedWork | Stage::Verifying
            )
        })
    }
}

pub(crate) fn step_dots(state: &State, active: &str, kit: &Kit) -> El {
    let wiring = state.is_local_backend();
    let slides = if state.startup.has_wiring_steps() && !state.startup.data_setup_active() {
        DataSlides::for_observed_work(state)
    } else {
        state.startup.data_slides
    };
    let keys = [
        wiring.then_some(startup::CONNECTION_DOT),
        wiring.then_some(startup::REGISTRATION_DOT),
        wiring.then_some(startup::HEALTH_CHECKS_DOT),
        slides.includes(Step::Seed).then_some(Step::Seed.key()),
        slides.includes(Step::Index).then_some(Step::Index.key()),
        state.supports_inbox_qa().then_some(INFERENCE_DOT),
        Some(CHECKS_DOT),
    ];
    let total = keys.iter().flatten().count();
    let mut current = 1;
    let mut dots = rune::row().key("startup-data-dots").center_y().gap(7.0);
    for (index, key) in keys.into_iter().flatten().enumerate() {
        if key == active {
            current = index + 1;
        }
        dots = dots.child(
            rune::el()
                .key(key)
                .size(7.0, 7.0)
                .radius(8.0)
                .fill(if key == active {
                    kit.c.icon_accent
                } else {
                    kit.c.border_strong
                })
                .a11y_hidden(),
        );
    }
    dots.role("status")
        .label(format!("Step {current} of {total}"))
}

fn panel(kit: &Kit, title: &str) -> El {
    rune::col()
        .key("startup-data-panel")
        .w_full()
        .max_w(510.0)
        .min_w(0.0)
        .gap(9.0)
        .padding(16.0)
        .radius(10.0)
        .fill(kit.c.layer_01)
        .border(1.0, crate::theme::hex(crate::theme::BORDER_SUBTLE))
        .child(
            views::heading(title, kit)
                .key("startup-data-stage")
                .role("heading")
                .focusable(true)
                .tab_index(-1),
        )
}

pub(crate) fn checks_view(state: &State, preferences: ViewPreferences, kit: &Kit) -> El {
    if crate::inbox_qa::setup_choice(state) {
        let panel = panel(kit, "Backend inference")
            .child(views::quiet(
                "Optional answers about workstation health in the Inbox.",
                kit,
            ))
            .child(crate::inbox_qa::settings_content(state, kit, preferences))
            .child(
                ActionButton::new("Continue to Workstation checks")
                    .key("startup-inbox-qa-continue")
                    .finish(MetalFinish::GoldCta)
                    .on_press_tx(crate::inbox_qa::continue_setup)
                    .build(kit, preferences, None),
            );
        return layout(
            panel,
            setup_sequence(state, None, preferences, kit),
            step_dots(state, INFERENCE_DOT, kit),
        );
    }
    let panel = panel(kit, "Workstation checks")
        .py(8.0)
        .gap(6.0)
        .child(crate::startup_checks::content(state, preferences, kit))
        .child(
            ActionButton::new("Open Workstation")
                .key("startup-data-continue")
                .finish(MetalFinish::GoldCta)
                .on_press_tx(State::finish_startup_checks)
                .build(kit, preferences, None),
        );
    layout(
        panel,
        setup_sequence(state, None, preferences, kit),
        step_dots(state, CHECKS_DOT, kit),
    )
}

fn data_stage_checklist(
    state: &State,
    step: Step,
    stage: &Stage<'_>,
    csv_validated: bool,
    preferences: ViewPreferences,
    kit: &Kit,
) -> El {
    let (work, mut result) = match stage {
        Stage::Observing => (CheckStatus::Pending, CheckStatus::Checking),
        Stage::Action if step == Step::Index && state.maintenance.pending_reindex.is_some() => {
            (CheckStatus::NeedsAttention, CheckStatus::Pending)
        }
        Stage::Action | Stage::AwaitingCatalog => (CheckStatus::Pending, CheckStatus::Pending),
        Stage::Working(_)
            if state
                .maintenance
                .operation_job(step.operation())
                .is_some_and(|record| record.poll_error.is_some()) =>
        {
            (CheckStatus::Unavailable, CheckStatus::Pending)
        }
        Stage::Working(_) | Stage::ObservedWork => (CheckStatus::Checking, CheckStatus::Pending),
        Stage::Verifying => (CheckStatus::Passed, CheckStatus::Verifying),
        Stage::Failed(_) => (CheckStatus::Failed, CheckStatus::Pending),
        Stage::Complete => (CheckStatus::Passed, CheckStatus::Passed),
    };
    if let Some(check) = current_check(state, step) {
        match check.state {
            ReadinessState::Unavailable => result = CheckStatus::Unavailable,
            ReadinessState::Degraded => result = CheckStatus::NeedsAttention,
            _ => {}
        }
    }
    let rows = match step {
        Step::Seed => {
            let csv = if csv_validated
                || (work == CheckStatus::Passed && saved_dataset(state, step).is_some())
            {
                CheckStatus::Passed
            } else if state.maintenance.github_downloading
                || matches!(state.maintenance.upload, Load::Loading)
            {
                CheckStatus::Checking
            } else if matches!(state.maintenance.upload, Load::Failed(_)) {
                CheckStatus::Failed
            } else if work == CheckStatus::Passed {
                CheckStatus::Unavailable
            } else {
                CheckStatus::Pending
            };
            [
                ("startup-seed-check-csv", "CSV validation", csv),
                ("startup-seed-check-job", "Seed job", work),
                ("startup-seed-check-catalog", "Verify actor catalog", result),
            ]
        }
        Step::Index => {
            let catalog = match current_check(state, Step::Seed).map(|check| check.state) {
                _ if operation_active(state, Step::Seed) => CheckStatus::Checking,
                Some(ReadinessState::Ready) => CheckStatus::Passed,
                Some(ReadinessState::Unavailable) => CheckStatus::Unavailable,
                Some(ReadinessState::Degraded) => CheckStatus::NeedsAttention,
                None if state.readiness_pending() => CheckStatus::Checking,
                _ => CheckStatus::Pending,
            };
            [
                ("startup-index-check-catalog", "Actor catalog", catalog),
                ("startup-index-check-build", "Index build", work),
                ("startup-index-check-search", "Verify actor search", result),
            ]
        }
    };
    startup_checks::list("startup-data-stage-checklist", "Setup progress").children(
        rows.into_iter()
            .map(|(key, title, status)| startup_checks::row(key, title, status, preferences, kit)),
    )
}

pub(crate) fn view(state: &State, step: Step, preferences: ViewPreferences, kit: &Kit) -> El {
    let stage = stage(state, step);
    let dots = step_dots(state, step.key(), kit);
    let sequence = setup_sequence(state, Some(step), preferences, kit);
    let csv_input = step
        .csv_input(&state.startup.csv_inputs)
        .filter(|_| matches!(stage, Stage::Action | Stage::Failed(_)));
    let upload = validated_csv(state, step);
    let mut panel = panel(kit, step.title());
    if csv_input.is_none() {
        panel = panel.children(input_summary(state, step, kit));
    }

    if let Some(confirmation) = state
        .maintenance
        .confirmation
        .as_ref()
        .filter(|confirmation| confirmation.operation == step.operation())
    {
        return layout(
            panel.child(backend_views::maintenance_confirmation_content(
                state,
                confirmation,
                preferences,
                kit,
            )),
            sequence,
            dots,
        );
    }
    if let Some(prompt) = &state.maintenance.csv_path_prompt {
        return layout(
            panel.child(backend_views::csv_path_prompt(
                prompt,
                preferences,
                kit,
                None,
            )),
            sequence,
            dots,
        );
    }
    panel = panel.child(data_stage_checklist(
        state,
        step,
        &stage,
        upload.is_some(),
        preferences,
        kit,
    ));
    if step == Step::Index {
        if let Some(unresolved) =
            backend_views::unresolved_actor_reindex_banner(state, preferences, kit)
        {
            return layout(
                panel.child(unresolved).child(later(preferences, kit)),
                sequence,
                dots,
            );
        }
    }

    let preflight = state.maintenance_preflight(step.operation());
    let progress = match stage {
        Stage::Working(detail) => Some(job_progress(
            detail.map(|detail| &detail.job),
            preferences,
            kit,
        )),
        Stage::Failed(detail) => {
            let job = &detail.job;
            let fallback = if job.error_summary.is_empty() {
                "The backend did not complete this step."
            } else {
                job.error_summary.as_str()
            };
            let explanation = pressure_explanation(&job.error_summary).unwrap_or(fallback);
            Some(status(
                if job.state == "cancelled" {
                    "Cancelled"
                } else {
                    "Needs attention"
                },
                explanation,
                StatusTone::Warn,
                false,
                preferences,
                kit,
            ))
        }
        Stage::Observing
        | Stage::Action
        | Stage::AwaitingCatalog
        | Stage::ObservedWork
        | Stage::Verifying
        | Stage::Complete => None,
    };
    panel = panel.children(progress);
    if step == Step::Seed {
        if let Stage::Working(Some(detail)) | Stage::Failed(detail) = stage {
            panel = panel.children(committed_seed_progress(state, detail).map(|progress| {
                views::quiet(progress, kit)
                    .key("startup-seed-committed")
                    .role("status")
            }));
        }
    }
    if let Some(input) = csv_input {
        panel = panel.children(csv_source_controls(step, input, kit));
        if let Some(upload) = upload {
            let detail = upload.row_count.map_or_else(
                || format!("{} · CSV validated", input.source.label()),
                |rows| {
                    format!(
                        "{} · {} rows validated",
                        input.source.label(),
                        format_row_count(rows)
                    )
                },
            );
            panel = panel.child(
                status(
                    if matches!(stage, Stage::Failed(_)) {
                        "CSV validated"
                    } else {
                        "Ready to seed"
                    },
                    detail,
                    StatusTone::Ok,
                    false,
                    preferences,
                    kit,
                )
                .key("startup-csv-ready"),
            );
        }
    }

    if let Some(record) = state.maintenance.operation_job(step.operation()) {
        if let Some(error) = &record.poll_error {
            panel = panel.child(views::quiet(
                format!("Job status unavailable: {error}"),
                kit,
            ));
        }
    }
    if let Load::Failed(error) = &state.maintenance.upload {
        panel = panel.child(views::line(error.clone(), kit).role("alert"));
    }
    if let Load::Failed(error) = &state.maintenance.receipt {
        panel = panel.child(views::line(error.clone(), kit).role("alert"));
    }
    if let Some(message) = &state.maintenance.message {
        panel = panel.child(views::quiet(message.clone(), kit).role("status"));
    }
    let downloading = state.maintenance.github_downloading;
    let validating = matches!(state.maintenance.upload, Load::Loading);
    if csv_input.is_some() && (downloading || validating) {
        let detail = if downloading {
            "Downloading the CSV from GitHub…"
        } else if csv_input.is_some_and(|input| input.source == CsvSource::Bundled) {
            "Validating the bundled CSV…"
        } else {
            "Validating the actor CSV…"
        };
        panel = panel.child(
            views::quiet(detail, kit)
                .key("startup-csv-progress")
                .role("status"),
        );
    }

    let mut actions = rune::row().w_full().flex_wrap().gap(8.0);
    match stage {
        Stage::Complete => {
            actions = actions.child(
                ActionButton::new("Continue")
                    .key("startup-data-continue")
                    .finish(MetalFinish::GoldCta)
                    .on_press_tx(State::next_startup_data_step)
                    .build(kit, preferences, None),
            );
        }
        Stage::Action | Stage::Failed(_) => {
            let choose_catalog = step == Step::Index && step.reset(state);
            let blocked = if choose_catalog {
                (state.readiness_pending()
                    || current_check(state, Step::Seed)
                        .is_none_or(|check| check.state != ReadinessState::Ready))
                .then(|| "Wait for the backend to verify the current actor catalog.".into())
            } else {
                preflight.disabled_reason()
            };
            let primary = if choose_catalog {
                "Use current catalog"
            } else if upload.is_some() {
                step.validated_action()
            } else if csv_input.is_some_and(|input| input.source == CsvSource::Bundled) {
                if matches!(state.maintenance.upload, Load::Failed(_)) {
                    "Retry validation"
                } else {
                    "Validate bundled CSV"
                }
            } else if csv_input.is_some_and(|input| input.source == CsvSource::Github) {
                "Download & validate"
            } else {
                step.action()
            };
            let busy = blocked.is_some() || downloading || validating || state.maintenance.busy();
            let validated = upload.is_some();
            let source = csv_input.map(|input| input.source);
            actions = actions.child(
                ActionButton::new(primary)
                    .key(PRIMARY)
                    .finish(MetalFinish::GoldCta)
                    .disabled(busy)
                    .on_press_tx(move |state: &mut State, tx| {
                        let current_source = step
                            .csv_input(&state.startup.csv_inputs)
                            .map(|input| input.source);
                        if source == current_source
                            && validated == validated_csv(state, step).is_some()
                        {
                            step.start(state, tx);
                        }
                    })
                    .build(kit, preferences, None),
            );
            if csv_input.is_some() && upload.is_none() && saved_dataset(state, step).is_some() {
                actions = actions.child(
                    ActionButton::new("Use saved CSV")
                        .key("startup-csv-saved")
                        .finish(MetalFinish::Gunmetal)
                        .disabled(busy)
                        .on_press_tx(move |state: &mut State, tx| step.use_saved_csv(state, tx))
                        .build(kit, preferences, None),
                );
            }
            if upload.is_some() && csv_input.is_some_and(|input| input.source == CsvSource::Local) {
                actions = actions.child(
                    ActionButton::new("Choose another CSV")
                        .key("startup-csv-change-local")
                        .finish(MetalFinish::Gunmetal)
                        .disabled(busy)
                        .on_press_tx(move |state: &mut State, tx| {
                            state.clear_startup_csv(step);
                            step.start(state, tx);
                        })
                        .build(kit, preferences, None),
                );
            }
            if let Some(reason) = blocked.filter(|_| !downloading && !validating) {
                panel = panel.child(views::quiet(reason, kit));
                actions = actions.child(refresh(state, preferences, kit));
            }
        }
        Stage::AwaitingCatalog => {
            panel = panel.child(views::quiet(
                "The index can start after the actor catalog is ready.",
                kit,
            ));
            actions = actions.child(refresh(state, preferences, kit));
        }
        Stage::Observing | Stage::ObservedWork | Stage::Verifying => {
            actions = actions.child(refresh(state, preferences, kit));
        }
        Stage::Working(_) => {
            if state
                .maintenance
                .operation_job(step.operation())
                .is_some_and(|record| record.poll_error.is_some())
            {
                actions = actions.child(refresh(state, preferences, kit));
            }
        }
    }
    if !matches!(stage, Stage::Complete) {
        actions = actions.child(later(preferences, kit));
    }
    layout(panel.child(actions), sequence, dots)
}

fn format_row_count(rows: u64) -> String {
    let mut count = rows.to_string();
    let mut separator = count.len();
    while separator > 3 {
        separator -= 3;
        count.insert(separator, ',');
    }
    count
}

fn csv_source_controls(step: Step, input: &CsvInput, kit: &Kit) -> Option<El> {
    let sources: Vec<_> = [CsvSource::Local, CsvSource::Bundled, CsvSource::Github]
        .into_iter()
        .filter(|source| match source {
            CsvSource::Local => true,
            CsvSource::Bundled => matches!(input.bundle, Bundle::Available { .. }),
            CsvSource::Github => cfg!(any(test, feature = "native-github-dataset")),
        })
        .collect();
    if sources.len() <= 1 {
        return None;
    }
    let labels: Vec<_> = sources.iter().map(|source| source.label()).collect();
    let options = SegmentedControl::new(
        &labels,
        sources.iter().position(|&source| source == input.source),
    )
    .label("CSV source")
    .fit()
    .on_pick(move |state: &mut State, index, _| {
        if let Some(&source) = sources.get(index) {
            state.startup_csv_source(step, source);
        }
    })
    .build(kit)
    .key("startup-csv-source");
    let url = (input.source == CsvSource::Github).then(|| {
        TextInput::new(&input.url)
            .label("GitHub CSV URL")
            .placeholder("https://raw.githubusercontent.com/…")
            .width(None)
            .on_edit(move |state: &mut State| {
                &mut step
                    .csv_input_mut(&mut state.startup.csv_inputs)
                    .expect("only dataset steps render a source URL")
                    .url
            })
            .on_change(move |state: &mut State, _| {
                if state.startup.data_step == Some(step) {
                    state.clear_startup_csv(step);
                }
            })
            .build(kit)
            .key("startup-csv-github-url")
    });
    Some(
        rune::col()
            .w_full()
            .min_w(0.0)
            .gap(6.0)
            .child(options)
            .children(url),
    )
}

fn layout(panel: El, sequence: El, dots: El) -> El {
    rune::col()
        .key("startup-data-setup")
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .child(
            rune::col()
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .scroll(rune::ScrollSpec {
                    stick_bottom: false,
                })
                .child(
                    rune::col()
                        .w_full()
                        .min_h(0.0)
                        .items_center()
                        .px(18.0)
                        .pt(6.0)
                        .pb(18.0)
                        .child(panel)
                        .child(sequence)
                        .child(rune::row().w_full().center().pt(10.0).child(dots)),
                ),
        )
}

fn input_summary(state: &State, step: Step, kit: &Kit) -> Option<El> {
    if step.reset(state) {
        return Some(if step == Step::Index {
            catalog_hash(state).map_or_else(
                || views::quiet("Choose the current actor catalog to continue.", kit),
                |hash| {
                    views::quiet(format!("Choose actor catalog · {}", &hash[..12]), kit)
                        .description(format!("Current actor catalog SHA-256 {hash}"))
                },
            )
        } else {
            views::quiet(
                "Input selection cleared. Existing backend data is kept.",
                kit,
            )
        });
    }
    if let Some(dataset) = saved_dataset(state, step) {
        return Some(
            views::quiet(format!("Saved CSV · {}", &dataset.sha256[..12]), kit)
                .description(format!("Saved CSV SHA-256 {}", dataset.sha256)),
        );
    }
    catalog_hash(state).map(|hash| {
        views::quiet(format!("Current actor catalog · {}", &hash[..12]), kit)
            .description(format!("Current actor catalog SHA-256 {hash}"))
    })
}

fn setup_sequence(
    state: &State,
    current: Option<Step>,
    preferences: ViewPreferences,
    kit: &Kit,
) -> El {
    let mut sequence = rune::row()
        .key("startup-data-sequence")
        .max_w(510.0)
        .w_full()
        .pt(10.0)
        .flex_wrap()
        .gap(5.0);
    for step in Step::ALL
        .into_iter()
        .filter(|&step| state.startup.data_slides.includes(step))
    {
        let complete = matches!(stage(state, step), Stage::Complete);
        let label = format!(
            "{} · {}",
            step.short_name(),
            if Some(step) == current {
                "Current"
            } else if complete {
                "Ready"
            } else {
                "Pending"
            }
        );
        sequence = sequence.child(
            rune::row()
                .min_w(0.0)
                .gap(4.0)
                .child(
                    ActionButton::new(label)
                        .key(step.review_key())
                        .finish(MetalFinish::Gunmetal)
                        .disabled(!can_review(state, step) || Some(step) == current)
                        .on_press_tx(move |state: &mut State, tx| {
                            state.review_startup_input(step, tx)
                        })
                        .build(kit, preferences, None),
                )
                .child(
                    ActionButton::new(step.reset_label())
                        .key(step.reset_key())
                        .finish(MetalFinish::ObsidianPill)
                        .disabled(!can_reset(state, step))
                        .on_press_tx(move |state: &mut State, tx| {
                            state.reset_startup_input(step, tx)
                        })
                        .build(kit, preferences, None),
                ),
        );
    }
    sequence
}

fn later(preferences: ViewPreferences, kit: &Kit) -> El {
    ActionButton::new("Finish later")
        .key("startup-data-skip")
        .finish(MetalFinish::Gunmetal)
        .on_press_tx(State::finish_startup_data_later)
        .build(kit, preferences, None)
}

fn refresh(state: &State, preferences: ViewPreferences, kit: &Kit) -> El {
    ActionButton::new("Refresh checks")
        .key("startup-data-refresh")
        .disabled(state.readiness_pending())
        .finish(MetalFinish::Gunmetal)
        .on_press(State::refresh_startup_data)
        .build(kit, preferences, None)
}

fn status(
    title: &'static str,
    detail: impl Into<String>,
    tone: StatusTone,
    busy: bool,
    preferences: ViewPreferences,
    kit: &Kit,
) -> El {
    rune::col()
        .key("startup-data-progress")
        .role("status")
        .w_full()
        .min_w(0.0)
        .gap(4.0)
        .child(
            rune::row()
                .center_y()
                .gap(6.0)
                .children(busy.then(|| ui_components::status_dot(tone, preferences)))
                .child(ui_components::status_pill(title, None, tone, kit)),
        )
        .child(views::quiet(detail, kit))
}

fn job_progress(job: Option<&backend::ActivityJob>, preferences: ViewPreferences, kit: &Kit) -> El {
    let Some(job) = job else {
        return status(
            "Waiting for job status",
            "The backend has not yet reported work progress.",
            StatusTone::Info,
            true,
            preferences,
            kit,
        );
    };
    if matches!(job.state.as_str(), "queued" | "retry_scheduled") {
        if let Some(explanation) = pressure_explanation(&job.error_summary) {
            return status(
                "Retrying",
                explanation,
                StatusTone::Warn,
                true,
                preferences,
                kit,
            );
        }
    }
    let title = match job.state.as_str() {
        "running" => "Running",
        "retry_scheduled" => "Retrying",
        _ => "Queued",
    };
    let mut detail = job.phase.trim().to_owned();
    if let Some(done) = job.progress_done.filter(|done| *done >= 0) {
        if !detail.is_empty() {
            detail.push_str(" · ");
        }
        match job
            .progress_total
            .filter(|total| *total > 0 && *total >= done)
        {
            Some(total) => detail.push_str(&format!("{done} / {total} processed")),
            None => detail.push_str(&format!("{done} processed")),
        }
    }
    status(title, detail, StatusTone::Info, true, preferences, kit)
}

#[cfg(test)]
mod tests {
    use fleetworks_contracts::connect::RegistrationReceipt;
    use fleetworks_workstation::native::BackendKind;
    use rune::Env;
    use rune::Runtime;

    use super::*;

    fn registered_data(seed: ReadinessState, index: ReadinessState) -> State {
        let mut state = crate::tests::registered_fixture();
        let context = state
            .registration
            .context
            .as_ref()
            .expect("registered fixture");
        let checks = [
            (ReadinessCheckId::ActorCatalog, seed),
            (ReadinessCheckId::ActorSearch, index),
        ]
        .into_iter()
        .map(|(id, state)| {
            let mut check = ReadinessCheck::new(
                id,
                state,
                "fixture_observation".into(),
                String::new(),
                "2026-11-27T00:00:00Z".into(),
            );
            check.fresh_for_ms = 60_000;
            check
        })
        .collect();
        let mut readiness = WorkstationReadiness::new(
            context.deployment_id.clone(),
            "2026-11-27T00:00:00Z".into(),
            checks,
        );
        readiness.registration = Some(RegistrationReceipt::new(
            context.deployment_id.clone(),
            context.workstation_id.clone(),
            context.registration_id.clone(),
            context.workstation_ip,
            context.registration_revision,
        ));
        state.readiness = Load::Ready(readiness);
        state.readiness_received_at = Some(0.0);
        state.startup = startup::Startup::cold_launch();
        state.setup_gateway = true;
        state.backend_kind = BackendKind::Local;
        state.connection.origin = TextEdit::new("local://embedded");
        state
    }

    #[test]
    fn startup_hides_seed_when_registered_seed_or_index_work_is_active() {
        use ReadinessState::{NotReady, Ready, Running};

        for (seed, index) in [
            (Running, NotReady),
            (Running, Ready),
            (NotReady, Running),
            (Running, Running),
        ] {
            let mut state = registered_data(seed, index);
            state.fixture_show_embedded_health_checks();
            let mut runtime = Runtime::new(
                state,
                crate::app::view,
                (640.0, 480.0),
                Env {
                    system_dark: true,
                    scale: 1.0,
                },
            );
            runtime.reduced_motion = true;
            runtime.frame(0.0);
            assert!(runtime.find("startup-stage-embedded-health").is_some());
            assert!(runtime.find(Step::Seed.key()).is_none());
            runtime.update(|state, _| state.enter_startup_data());
            runtime.frame(0.02);
            assert_eq!(runtime.state.startup.data_step, Some(Step::Index));
            assert!(runtime.find("startup-stage-seed").is_none());
            assert!(runtime.find(Step::Seed.key()).is_none());
            assert!(runtime.find(Step::Seed.review_key()).is_none());
            assert!(runtime.find(Step::Seed.reset_key()).is_none());
            assert!(runtime.find(PRIMARY).is_none());
            assert!(runtime.find("startup-stage-index").is_some());
            let tree = runtime.accessibility_json().to_string();
            if index == Running {
                assert!(tree.contains("Index build: Checking"));
            } else {
                assert!(tree.contains("Actor catalog: Checking"));
                assert!(tree.contains("Index build: Pending"));
            }
            runtime.update(|state, tx| {
                let Load::Ready(readiness) = &mut state.readiness else {
                    panic!("registered fixture readiness");
                };
                readiness.checks[0].state = Ready;
                readiness.checks[1].state = Ready;
                state.next_startup_data_step(tx);
            });
            runtime.frame(0.04);
            assert!(runtime.find(Step::Seed.key()).is_none());
            assert!(runtime.find("startup-stage-checks").is_some());
        }
    }

    #[test]
    fn startup_needs_scoped_current_evidence_to_skip_seed() {
        use ReadinessState::{NotReady, Running};

        let mut state = registered_data(Running, Running);
        state.readiness_received_at = None;
        state.enter_startup_data();
        assert_eq!(state.startup.data_step, Some(Step::Seed));
        state.readiness_received_at = Some(0.0);
        let Load::Ready(readiness) = &mut state.readiness else {
            panic!("registered fixture readiness");
        };
        readiness
            .registration
            .as_mut()
            .expect("fixture registration")
            .revision += 1;
        state.enter_startup_data();
        assert_eq!(state.startup.data_step, Some(Step::Seed));

        let mut state = registered_data(NotReady, NotReady);
        let operation = Step::Index.operation();
        let id = "fixture-index";
        state
            .maintenance
            .set_operation_job(operation, id.into(), false);
        state.enter_startup_data();
        assert_eq!(state.startup.data_step, Some(Step::Seed));
        let detail = serde_json::from_value(serde_json::json!({
            "job": {
                "id": id,
                "app": "brain",
                "title": "Actor search index",
                "operation": Step::Index.job_operation(),
                "state": "running",
                "phase": "indexing",
                "error_summary": "",
                "private_details_available": true,
                "activity_actions_available": false,
                "status_only": false
            },
            "stages": [],
            "private_details_available": true,
            "activity_actions_available": false
        }))
        .expect("tracked index detail");
        state
            .maintenance
            .update_operation_job_detail(operation, id, detail);
        state.enter_startup_data();
        assert_eq!(state.startup.data_step, Some(Step::Index));
        state.invalidate_origin();
        assert_eq!(DataSlides::for_observed_work(&state), DataSlides::All);
    }
}
