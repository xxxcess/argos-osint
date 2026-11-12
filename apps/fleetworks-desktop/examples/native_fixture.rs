//! Development-only same-view fixtures. No real credentials, home, gateway or browser.

#[cfg(target_os = "macos")]
mod native {
    mod activation_scenes;
    mod appearance_scenes;
    mod job_action_scenes;
    mod lifecycle_scenes;
    mod readiness_scenes;
    mod submission_scenes;

    use fleetworks_contracts::capabilities::Capabilities;
    use fleetworks_contracts::connect::RegistrationReceipt;
    use fleetworks_contracts::context::ActivityScope;
    use fleetworks_contracts::context::JobOrigin;
    use fleetworks_contracts::context::OriginSource;
    use fleetworks_contracts::context::Permission;
    use fleetworks_contracts::context::RegisteredContext;
    use fleetworks_contracts::readiness::DatasetKind;
    use fleetworks_contracts::readiness::DatasetRef;
    use fleetworks_contracts::readiness::MaintenanceOperation;
    use fleetworks_contracts::readiness::MaintenanceReceipt;
    use fleetworks_contracts::readiness::ReadinessAction;
    use fleetworks_contracts::readiness::ReadinessActionId;
    use fleetworks_contracts::readiness::ReadinessCheck;
    use fleetworks_contracts::readiness::ReadinessCheckId;
    use fleetworks_contracts::readiness::ReadinessState;
    use fleetworks_contracts::readiness::WorkstationReadiness;
    use fleetworks_contracts::search::ActorSearchHit;
    use fleetworks_contracts::search::ActorSearchResponse;
    use fleetworks_contracts::search::IndexState;
    use fleetworks_contracts::search::IndexStatus;
    use fleetworks_desktop::app;
    use fleetworks_desktop::app::Intent;
    use fleetworks_desktop::app::Load;
    use fleetworks_desktop::backend::AccountMetadata;
    use fleetworks_desktop::backend::ActivityJob;
    use fleetworks_desktop::backend::ActivityJobs;
    use fleetworks_desktop::backend::Article;
    use fleetworks_desktop::backend::ArticlePage;
    use fleetworks_desktop::backend::JobDetail;
    use fleetworks_desktop::backend::JobLogs;
    use fleetworks_desktop::backend::LogEntry;
    use fleetworks_desktop::backend::ModelDefaults;
    use fleetworks_desktop::backend::ProfileSnapshot;
    use fleetworks_desktop::backend::ReadKind;
    use fleetworks_desktop::backend::ToolCatalog;
    use fleetworks_desktop::backend::ToolDefinition;
    use fleetworks_desktop::brain::ActorDossier;
    use fleetworks_desktop::configuration::ConfigurationSaved;
    use fleetworks_desktop::connection::ConnectionStatus;
    use fleetworks_desktop::registration::InterfaceSnapshot;
    use fleetworks_desktop::registration::NativeInterface;
    use fleetworks_desktop::transfer::TransferConfirmation;
    use fleetworks_desktop::workspaces::WorkspaceCommand;
    use fleetworks_desktop::workspaces::WorkspaceRow;
    use fleetworks_desktop::workspaces::WorkspaceSnapshot;
    use fleetworks_desktop::Page;
    use fleetworks_desktop::State;
    use fleetworks_workspace::native::WorkspaceState;
    use rune::AppSpec;
    use rune::TextEdit;

    fn init() -> State {
        State::fixture()
    }

    fn connected(state: &mut State) {
        let capabilities: Capabilities = serde_json::from_value(serde_json::json!({
        "api_version":1,
        "features":["connect-registry-v1"],
        "deployment_id":"synthetic-deployment",
        "configuration_epoch":"synthetic-fresh-config-v1",
        "connect":{"version":1,"registration_required":true},
        "workspace_control":{"version":1,"purpose":"registration_and_activity"},
        "resource_scopes":{"atlas":"backend","brain":"backend","tools":"backend","models":"backend","profile":"backend","native_jobs_logs":"workstation_triggered_activity"},
        "search":{"actors":{"version":1,"engine":"tantivy"}},
        "permissions":["read_backend","register_workstation","read_workstation_activity","maintain_backend","manage_configuration","submit_interactive","rebind_workstation"],
        "links":{"intel_web":{"url":"http://127.0.0.1:3000/","connect_url":"http://127.0.0.1:3000/connect"}}
    })).expect("the synthetic capability fixture uses the current contract");
        state.connection.capabilities = Some(capabilities);
        state.connection.status = ConnectionStatus::Connected;
        state.fixture_effect = Some(reply);
    }

    fn workspace_rows() -> Vec<WorkspaceRow> {
        ["default", "research"]
            .into_iter()
            .enumerate()
            .map(|(index, name)| WorkspaceRow {
                name: name.into(),
                path: format!("/fixture/home/.fleetworks/{name}").into(),
                marker: format!("synthetic-workspace-{index}"),
                protected_default: index == 0,
                state: WorkspaceState::Local,
                bindings: Vec::new(),
            })
            .collect()
    }

    fn workspaces_ready(state: &mut State) {
        connected(state);
        let rows = workspace_rows();
        state.change_workspace(rows[0].clone());
        state.workspaces.snapshot = Load::Ready(WorkspaceSnapshot {
            root: "/fixture/home/.fleetworks".into(),
            installation_marker: "synthetic-installation".into(),
            rows,
            operations: Vec::new(),
            registration_attempts: Vec::new(),
            rebind_attempts: Vec::new(),
        });
        state.page = Page::Workspaces;
    }

    fn registered(state: &mut State) {
        workspaces_ready(state);
        let interface = NativeInterface {
            name: "fixture-vpn".into(),
            ip: "192.0.2.42".parse().expect("fixture IP"),
        };
        state.registration.interfaces = Load::Ready(InterfaceSnapshot {
            interfaces: vec![interface.clone()],
            route_suggestion: Some(interface.ip),
        });
        state.registration.selected_interface = Some(interface);
        let receipt = RegistrationReceipt::new(
            "synthetic-deployment".into(),
            "synthetic-node".into(),
            "synthetic-workspace".into(),
            "synthetic-registration".into(),
            "192.0.2.42:default".parse().expect("fixture Connect ID"),
            1,
        );
        state.registration.context = Some(
            RegisteredContext::new(
                receipt.clone(),
                1,
                1,
                "synthetic-fresh-config-v1".into(),
                "synthetic-access-1".into(),
            )
            .expect("fixture registered context"),
        );
        state.registration.receipt = Load::Ready(receipt);
        state.page = Page::Workstation;
    }

    fn profile_ready(state: &mut State) {
        connected(state);
        state.backend.profile = Load::Ready(ProfileSnapshot {
            configuration_revision: 7,
            configuration_format: 1,
            accounts: vec![AccountMetadata {
                id: "synthetic-account".into(),
                provider: "local".into(),
                base_url: "http://127.0.0.1:11434".into(),
                model: "synthetic-model".into(),
                has_api_key: false,
                credential_reference: "account:synthetic-account".into(),
            }],
            tool_credentials: Vec::new(),
            setup_required: false,
            portable_schema_version: 4,
        });
        state.page = Page::Profile;
    }

    fn models_ready(state: &mut State) {
        connected(state);
        state.backend.models = Load::Ready(ModelDefaults {
            configuration_revision: 7,
            roles: [
                "recon_agent",
                "recon_decisions",
                "recon_analyst",
                "atlas_evaluator",
                "atlas_analyst",
                "brain_summary",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
            defaults: std::collections::BTreeMap::from([(
                "brain_summary".into(),
                serde_json::json!({"provider":"local","model":"synthetic-primary","account":"synthetic-account","fallbacks":[{"provider":"local","model":"synthetic-fallback","account":"synthetic-account"}]}),
            )]),
        });
        state.page = Page::Models;
    }

    fn model_edit_preview(state: &mut State) {
        models_ready(state);
        state.edit_model_role("brain_summary");
        state.configuration.model = TextEdit::new("synthetic-new-primary");
        state.preview_model_role(false);
    }

    fn account_edit_preview(state: &mut State) {
        profile_ready(state);
        state.edit_provider_account("synthetic-account");
        state
            .configuration
            .account_api_key
            .replace_text("synthetic-secret-never-rendered");
        state.preview_provider_account();
    }

    fn synthetic_portable() -> Vec<u8> {
        let roles: std::collections::BTreeMap<_, _> = [
            "recon_agent",
            "recon_decisions",
            "recon_analyst",
            "atlas_evaluator",
            "atlas_analyst",
            "brain_summary",
        ]
        .into_iter()
        .map(|role| (role, serde_json::json!({"primary":null,"fallbacks":[]})))
        .collect();
        serde_json::to_vec(&serde_json::json!({"schema_version":4,"accounts":{},"roles":roles,"tool_credentials":{}})).expect("synthetic portable fixture")
    }

    fn atlas_ready(state: &mut State) {
        connected(state);
        state.backend.article_day = TextEdit::new("2026-01-01");
        state.backend.articles = Load::Ready(ArticlePage {
            rows: vec![Article {
                id: "synthetic-article".into(),
                title: "Synthetic source report".into(),
                description: "A bounded native article-list fixture.".into(),
                source_name: "Fixture News".into(),
                published_at: "2026-01-01T00:00:00Z".into(),
                category: "pol".into(),
                categories: vec!["pol".into(), "mil".into()],
            }],
            next_cursor: None,
            revision: "synthetic-revision".into(),
        });
        state.page = Page::Atlas;
    }

    fn tools_ready(state: &mut State) {
        connected(state);
        state.backend.tools = Load::Ready(ToolCatalog {
            tools: vec![ToolDefinition {
                id: "synthetic-tool".into(),
                name: "Synthetic lookup".into(),
                category: "Fixture".into(),
                description: "A synthetic catalog entry; no provider call is made.".into(),
                inputs: vec!["email".into()],
                restrictions: "Fixture only".into(),
                timeout_seconds: 30,
            }],
        });
        state.backend.selected_tool = Some("synthetic-tool".into());
        state.page = Page::Tools;
    }

    fn jobs_ready(state: &mut State) {
        registered(state);
        let context = state
            .registration
            .context
            .as_ref()
            .expect("synthetic context");
        let job = ActivityJob {
            id: "synthetic-job".into(),
            app: "Workstation".into(),
            title: "Synthetic accepted seed".into(),
            operation: "actor_seed".into(),
            state: "running".into(),
            phase: "seeding".into(),
            progress_done: Some(250),
            progress_total: Some(1000),
            error_summary: String::new(),
            private_details_available: true,
            activity_actions_available: false,
            status_only: false,
            can_cancel: false,
            can_retry: false,
            action_reason_code: "not_observed".into(),
        };
        state.backend.jobs = Load::Ready(ActivityJobs {
            rows: vec![job.clone()],
            activity_scope: ActivityScope::for_workstation(context),
            relation_counts_available: false,
            activity_actions_available: false,
        });
        state.backend.selected_job = Some(job.id.clone());
        state.backend.detail = Load::Ready(JobDetail {
            job,
            maintenance_result: None,
            stages: Vec::new(),
            private_details_available: true,
            activity_actions_available: false,
        });
        state.backend.logs = Load::Ready(JobLogs {
            rows: vec![LogEntry {
                ts: "2026-01-01T00:00:00Z".into(),
                severity: "info".into(),
                message: "Synthetic committed batch observed".into(),
                job_id: "synthetic-job".into(),
            }],
            available: true,
            reason: None,
        });
        state.page = Page::Jobs;
    }

    fn maintenance_preview(state: &mut State) {
        registered(state);
        let page = state.page;
        profile_ready(state);
        state.page = page;
        let context = state
            .registration
            .context
            .as_ref()
            .expect("fixture context");
        let mut check = ReadinessCheck::new(
            ReadinessCheckId::ActorCatalog,
            ReadinessState::NotReady,
            "synthetic_not_seeded".into(),
            "Synthetic backend catalog is not seeded.".into(),
            "2026-01-01T00:00:00Z".into(),
        );
        check.fresh_for_ms = 30_000;
        check.actions.push(ReadinessAction::new(
            ReadinessActionId::SeedActors,
            Permission::MaintainBackend,
            true,
        ));
        let mut readiness = WorkstationReadiness::new(
            context.deployment_id.clone(),
            "2026-01-01T00:00:00Z".into(),
            vec![check],
        );
        if let Load::Ready(receipt) = &state.registration.receipt {
            readiness.registration = Some(receipt.clone());
        }
        state.readiness = Load::Ready(readiness);
        state.readiness_received_at = Some(0.0);
        state.maintenance.selected_dataset = Some(
            DatasetRef::new(
                "synthetic-dataset".into(),
                DatasetKind::ActorCatalog,
                "a".repeat(64),
                1024,
            )
            .expect("fixture dataset"),
        );
        state.preview_maintenance(MaintenanceOperation::SeedActors);
    }

    fn brain_ready(state: &mut State) {
        connected(state);
        state.page = Page::Brain;
        state.brain.query = TextEdit::new("Harbor");
        let mut index = IndexStatus::unknown("2026-01-01T00:00:00Z".into());
        index.state = IndexState::Ready;
        index.generation = Some("synthetic-generation-1".into());
        let hit = ActorSearchHit::new(
            "synthetic-actor-1".into(),
            "Harbor Union".into(),
            1,
            "synthetic-generation-1".into(),
        );
        state.brain.results = Load::Ready(ActorSearchResponse::new(vec![hit], index));
    }

    fn brain_dossier(state: &mut State) {
        brain_ready(state);
        populate_dossier(state);
    }

    fn populate_dossier(state: &mut State) {
        state.brain.selected_actor = Some("synthetic-actor-1".into());
        state.brain.showing_detail = true;
        state.brain.dossier = Load::Ready(ActorDossier {
            id: "synthetic-actor-1".into(),
            display_name: "Harbor Union".into(),
            subtype: "Synthetic organization".into(),
            identity_state: "active".into(),
            description: Some("A synthetic actor used only for native verification.".into()),
            aliases: vec!["Harbor".into()],
            metadata: Vec::new(),
            claims: Vec::new(),
            briefs: Vec::new(),
            inferences: Vec::new(),
            gaps: vec!["No additional records in this bounded synthetic fixture.".into()],
            truncated: false,
        });
    }

    fn reply(state: &mut State, intent: &Intent) {
        if activation_scenes::reply(state, intent) {
            return;
        }
        if lifecycle_scenes::reply(state, intent) {
            return;
        }
        if job_action_scenes::reply(state, intent) {
            return;
        }
        if submission_scenes::reply(state, intent) {
            return;
        }
        match intent {
            Intent::Dossier { .. } => populate_dossier(state),
            Intent::Search { .. } => {
                let revision = state.brain.query_revision();
                let mut index = IndexStatus::unknown("2026-01-01T00:00:00Z".into());
                index.state = IndexState::Ready;
                let hit = ActorSearchHit::new(
                    "synthetic-actor-1".into(),
                    "Harbor Union".into(),
                    1,
                    "synthetic-generation-1".into(),
                );
                state
                    .brain
                    .complete_search(revision, Ok(ActorSearchResponse::new(vec![hit], index)));
            }
            Intent::Workspace(WorkspaceCommand::Select(name)) => {
                if let Load::Ready(snapshot) = &state.workspaces.snapshot {
                    if let Some(row) = snapshot.rows.iter().find(|row| &row.name == name).cloned() {
                        state.change_workspace(row);
                    }
                }
            }
            Intent::BackendRead {
                kind: ReadKind::Profile,
            } => {
                let page = state.page;
                profile_ready(state);
                state.page = page;
            }
            Intent::Register => {
                let (Some(row), Some(interface)) = (
                    &state.workspaces.selected,
                    &state.registration.selected_interface,
                ) else {
                    return;
                };
                let connect = fleetworks_contracts::connect::ConnectId::new(
                    interface.ip,
                    row.name.parse().expect("synthetic workspace name"),
                )
                .expect("synthetic Connect ID");
                let receipt = RegistrationReceipt::new(
                    "synthetic-deployment".into(),
                    "synthetic-node".into(),
                    format!("synthetic-workspace-{}", row.name),
                    format!("synthetic-registration-{}", row.name),
                    connect,
                    1,
                );
                state.registration.context = Some(
                    RegisteredContext::new(
                        receipt.clone(),
                        1,
                        1,
                        "synthetic-fresh-config-v1".into(),
                        "synthetic-access-1".into(),
                    )
                    .expect("synthetic context"),
                );
                state.registration.receipt = Load::Ready(receipt);
            }
            Intent::Configuration { .. } => {
                state.configuration.result = Load::Ready(ConfigurationSaved {
                    saved: true,
                    configuration_revision: 8,
                });
                state.configuration.account_api_key.replace_text("");
                state.configuration.tool_primary.replace_text("");
                state.configuration.tool_fallback.replace_text("");
                if let Load::Ready(profile) = &mut state.backend.profile {
                    profile.configuration_revision = 8;
                }
                if let Load::Ready(models) = &mut state.backend.models {
                    models.configuration_revision = 8;
                }
            }
            Intent::PrepareProfileImport => {
                state.transfer.confirmation = Some(TransferConfirmation::Import {
                    expected_revision: 7,
                    origin_revision: state.origin_revision(),
                    file_name: "synthetic-profile.json".into(),
                    bytes: synthetic_portable(),
                });
            }
            Intent::ProfileTransfer { .. } => {
                state.transfer.result = Load::Ready(
                    "Synthetic transfer effect observed; no file or backend was changed".into(),
                )
            }
            Intent::Maintenance { operation } => {
                let Some(context) = &state.registration.context else {
                    return;
                };
                let origin = JobOrigin::new(
                    context,
                    OriginSource::Native,
                    "synthetic-maintenance-request".into(),
                    "2026-01-01T00:00:00Z".into(),
                    7,
                    Vec::new(),
                );
                state.maintenance.receipt = Load::Ready(MaintenanceReceipt::new(
                    "synthetic-job".into(),
                    *operation,
                    origin,
                    state.maintenance.selected_dataset.clone(),
                    false,
                ));
            }
            Intent::UploadDataset { kind } => {
                let dataset =
                    DatasetRef::new("synthetic-dataset".into(), *kind, "a".repeat(64), 1024)
                        .expect("synthetic dataset");
                state.maintenance.selected_dataset = Some(dataset.clone());
                state.maintenance.upload = Load::Ready(
                    serde_json::from_value(
                        serde_json::json!({"dataset":dataset,"expires_in_seconds":86400}),
                    )
                    .expect("synthetic upload receipt"),
                );
            }
            Intent::Datasets => {
                state.maintenance.datasets =
                    Load::Ready(fleetworks_desktop::maintenance::DatasetList {
                        rows: state.maintenance.selected_dataset.iter().cloned().collect(),
                        truncated: false,
                    });
            }
            Intent::BackendRead {
                kind: ReadKind::Models,
            } => {
                let page = state.page;
                models_ready(state);
                state.page = page;
            }
            Intent::BackendRead {
                kind: ReadKind::Articles,
            } => {
                let page = state.page;
                atlas_ready(state);
                state.page = page;
            }
            Intent::BackendRead {
                kind: ReadKind::Tools,
            } => {
                let page = state.page;
                tools_ready(state);
                state.page = page;
            }
            Intent::BackendRead {
                kind: ReadKind::Jobs | ReadKind::JobDetail | ReadKind::Logs,
            } => {
                let page = state.page;
                jobs_ready(state);
                state.page = page;
            }
            _ => {}
        }
    }

    pub fn run() {
        rune::run(AppSpec {
            init,
            scenes: &[
                ("activation-candidate-preview", activation_scenes::preview),
                ("activation-confirmation", activation_scenes::confirmation),
                ("activation-accepted", activation_scenes::accepted),
                ("activation-effect-uncertain", activation_scenes::uncertain),
                ("activation-completed", activation_scenes::completed),
                (
                    "activation-acceptance-unknown",
                    activation_scenes::acceptance_unknown,
                ),
                ("appearance-native", appearance_scenes::satin),
                ("appearance-matte", appearance_scenes::matte),
                (
                    "appearance-reduced-motion",
                    appearance_scenes::reduced_motion,
                ),
                ("appearance-material-fallback", appearance_scenes::fallback),
                ("workstation-observed", readiness_scenes::observed),
                ("workstation-connecting", readiness_scenes::connecting),
                ("workstation-unsupported", readiness_scenes::unsupported),
                ("workstation-stale", readiness_scenes::stale),
                ("workstation-checks-error", readiness_scenes::checks_error),
                (
                    "workstation-permission-denied",
                    readiness_scenes::permission_denied,
                ),
                ("workstation-partial-seed", readiness_scenes::partial_seed),
                (
                    "maintenance-partial-seed-progress",
                    readiness_scenes::partial_seed_progress,
                ),
                ("workstation-training", readiness_scenes::training),
                (
                    "workstation-inference-unverified",
                    readiness_scenes::inference_unverified,
                ),
                (
                    "workstation-inference-error",
                    readiness_scenes::inference_error,
                ),
                ("workstation-offline", |state| {
                    state.page = Page::Workstation
                }),
                ("atlas-offline", |state| state.page = Page::Atlas),
                ("atlas-articles-ready", atlas_ready),
                ("atlas-submit-preview", submission_scenes::atlas_preview),
                ("atlas-submit-accepted", submission_scenes::atlas_accepted),
                ("atlas-submit-uncertain", submission_scenes::atlas_uncertain),
                ("atlas-request-resolved", submission_scenes::atlas_resolved),
                ("brain-search-ready", brain_ready),
                ("brain-dossier", brain_dossier),
                ("brain-search-indexing", |state| {
                    brain_ready(state);
                    if let Load::Ready(response) = &mut state.brain.results {
                        response.index.state = IndexState::Indexing;
                    }
                }),
                ("brain-search-error", |state| {
                    state.page = Page::Brain;
                    state.brain.results = Load::Failed(
                        "Synthetic search unavailable; retry when the gateway recovers.".into(),
                    );
                }),
                ("tools-offline", |state| state.page = Page::Tools),
                ("tools-catalog-ready", tools_ready),
                ("tools-submit-preview", submission_scenes::tools_preview),
                ("tools-result-ready", submission_scenes::tools_result),
                ("tools-submission-shared", submission_scenes::tools_shared),
                ("tools-result-error", submission_scenes::tools_result_error),
                ("models-setup", |state| state.page = Page::Models),
                ("models-ready", models_ready),
                ("models-edit-preview", model_edit_preview),
                ("profile-setup", |state| state.page = Page::Profile),
                ("profile-ready", profile_ready),
                ("profile-account-preview", account_edit_preview),
                ("profile-export-confirm", |state| {
                    profile_ready(state);
                    state.preview_profile_export();
                }),
                ("profile-import-confirm", |state| {
                    profile_ready(state);
                    state.transfer.confirmation = Some(TransferConfirmation::Import {
                        expected_revision: 7,
                        origin_revision: state.origin_revision(),
                        file_name: "synthetic-profile.json".into(),
                        bytes: synthetic_portable(),
                    });
                }),
                ("workspaces-offline", |state| state.page = Page::Workspaces),
                ("workspaces-ready", workspaces_ready),
                ("registration-ready", registered),
                ("maintenance-preview", maintenance_preview),
                ("jobs-offline", |state| state.page = Page::Jobs),
                ("jobs-ready", jobs_ready),
                ("job-cancel-preview", job_action_scenes::cancel_preview),
                ("job-retry-preview", job_action_scenes::retry_preview),
                ("job-cancel-receipt", job_action_scenes::cancel_receipt),
                ("job-retry-receipt", job_action_scenes::retry_receipt),
                ("workspace-rebind-preview", lifecycle_scenes::rebind_preview),
                (
                    "workspace-rebind-uncertain",
                    lifecycle_scenes::rebind_uncertain,
                ),
                (
                    "workspace-retirement-pending",
                    lifecycle_scenes::retirement_pending,
                ),
                ("workspace-trash-recovery", lifecycle_scenes::trash_recovery),
                (
                    "maintenance-training-result",
                    lifecycle_scenes::training_result,
                ),
                ("maintenance-seed-result", lifecycle_scenes::seed_result),
                ("jobs-shared-status", |state| {
                    jobs_ready(state);
                    if let Load::Ready(detail) = &mut state.backend.detail {
                        detail.private_details_available = false;
                        detail.job.private_details_available = false;
                        detail.job.status_only = true;
                    }
                    state.backend.logs = Load::Ready(JobLogs {
                        rows: Vec::new(),
                        available: false,
                        reason: Some("Shared private logs unavailable".into()),
                    });
                }),
            ],
            ..app::spec()
        });
    }
}

#[cfg(target_os = "macos")]
fn main() {
    native::run();
}

#[cfg(not(target_os = "macos"))]
fn main() -> anyhow::Result<()> {
    anyhow::bail!(
        "The native fixture requires a real macOS Rune renderer; this host cannot provide native fixture evidence."
    )
}
