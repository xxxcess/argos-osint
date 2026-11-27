//! Development-only same-view fixtures. No real credentials, home, gateway or browser.

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod native {
    mod appearance_scenes;
    mod connection_profile_scenes;
    mod job_action_scenes;
    mod jobs_scenes;
    mod lifecycle_scenes;
    mod log_scenes;
    mod overhaul_scenes;
    mod readiness_scenes;
    mod redesign_scenes;
    mod startup_data_scenes;
    mod submission_scenes;

    use fleetworks_contracts::capabilities::Capabilities;
    use fleetworks_contracts::chat::ChatSourceKind;
    use fleetworks_contracts::chat::ChatSourceReference;
    use fleetworks_contracts::chat::QaAvailabilityReason;
    use fleetworks_contracts::chat::QaDecisionAvailabilityReason;
    use fleetworks_contracts::chat::QaDecisionPreference;
    use fleetworks_contracts::chat::QaDecisionSettings;
    use fleetworks_contracts::chat::QaFinding;
    use fleetworks_contracts::chat::QaGeneratorPreference;
    use fleetworks_contracts::chat::QaGeneratorSettings;
    use fleetworks_contracts::chat::QaModelTarget;
    use fleetworks_contracts::chat::QaPhase;
    use fleetworks_contracts::chat::QaSettings;
    use fleetworks_contracts::chat::QaTurn;
    use fleetworks_contracts::chat::QaTurnLimitation;
    use fleetworks_contracts::chat::QaTurnStage;
    use fleetworks_contracts::chat::QaTurnState;
    use fleetworks_contracts::chat::QA_MODEL_SETTINGS_FEATURE;
    use fleetworks_contracts::chat::QA_TRIAGE_FEATURE;
    use fleetworks_contracts::connect::RegistrationReceipt;
    use fleetworks_contracts::context::JobOrigin;
    use fleetworks_contracts::context::OriginSource;
    use fleetworks_contracts::context::Permission;
    use fleetworks_contracts::context::RegisteredContext;
    use fleetworks_contracts::qa_worker::QaContextUsage;
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
    use fleetworks_desktop::accessibility::FixtureAccessibility;
    use fleetworks_desktop::app;
    use fleetworks_desktop::app::Intent;
    use fleetworks_desktop::app::Load;
    use fleetworks_desktop::backend::AccountMetadata;
    use fleetworks_desktop::backend::ActivityJob;
    use fleetworks_desktop::backend::JobDetail;
    use fleetworks_desktop::backend::JobLogs;
    use fleetworks_desktop::backend::LogEntry;
    use fleetworks_desktop::backend::ModelDefaults;
    use fleetworks_desktop::backend::ProfileSnapshot;
    use fleetworks_desktop::backend::ReadKind;
    use fleetworks_desktop::backend::ToolCatalog;
    use fleetworks_desktop::backend::ToolCredentialMetadata;
    use fleetworks_desktop::backend::ToolDefinition;
    use fleetworks_desktop::brain::ActorDossier;
    use fleetworks_desktop::configuration::ConfigurationSaved;
    use fleetworks_desktop::connection::ConnectionStatus;
    use fleetworks_desktop::local_backend::BinaryDiscoveryStatus;
    use fleetworks_desktop::registration::InterfaceSnapshot;
    use fleetworks_desktop::registration::NativeInterface;
    use fleetworks_desktop::transfer::TransferConfirmation;
    use fleetworks_desktop::workspaces::WorkspaceCommand;
    use fleetworks_desktop::workspaces::WorkspaceRow;
    use fleetworks_desktop::workspaces::WorkspaceSnapshot;
    use fleetworks_desktop::Page;
    use fleetworks_desktop::State;
    use fleetworks_workstation::native::BackendKind;
    use fleetworks_workstation::native::WorkspaceState;
    use rune::AppSpec;
    use rune::TextEdit;

    fn init() -> State {
        let mut state = State::fixture();
        state.local_backend.binary_status =
            BinaryDiscoveryStatus::Ready("/fixture/bundle/argos-api-host".into());
        state
    }

    fn connected(state: &mut State) {
        let capabilities: Capabilities = serde_json::from_value(serde_json::json!({
        "api_version":1,
        "features":[fleetworks_contracts::capabilities::CONNECT_REGISTRY_FEATURE,"atlas-cycles","atlas-cycle-history-v1","tool-execution"],
        "deployment_id":"synthetic-deployment",
        "configuration_epoch":"synthetic-fresh-config-v1",
        "connect":{"version":fleetworks_contracts::connect::CONNECT_CONTRACT_VERSION,"registration_required":true},
        "workspace_control":{"version":fleetworks_contracts::connect::CONNECT_CONTRACT_VERSION,"purpose":"registration_and_activity"},
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
            "synthetic-registration".into(),
            Some("192.0.2.42".parse().expect("fixture IP")),
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
            catalog: serde_json::from_value(serde_json::json!([
                {"provider":"local","account":"synthetic-account","model":"synthetic-primary","efforts":[]},
                {"provider":"local","account":"synthetic-account","model":"synthetic-fallback","efforts":[]}
            ])).expect("synthetic catalog"),
            model_variants: Default::default(),
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

    fn tools_ready(state: &mut State) {
        registered(state);
        state.backend.profile = Load::Ready(ProfileSnapshot {
            configuration_revision: 1,
            configuration_format: 4,
            accounts: Vec::new(),
            tool_credentials: vec![ToolCredentialMetadata {
                provider: "newsdata".into(),
                primary_present: true,
                fallback_present: false,
            }],
            setup_required: false,
            portable_schema_version: 4,
        });
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
        jobs_scenes::install(state, vec![job.clone()]);
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
        readiness.configured_inputs.push(serde_json::from_value(serde_json::json!({
            "operation":"seed_actors",
            "dataset": DatasetRef::new("synthetic-dataset".into(), DatasetKind::ActorCatalog, "a".repeat(64), 1024).expect("configured fixture dataset"),
            "configuration_revision": state.configuration_revision().expect("fixture configuration")
        })).expect("configured operation input"));
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
        if log_scenes::reply(state, intent) {
            return;
        }
        if connection_profile_scenes::reply(state, intent) {
            return;
        }
        if lifecycle_scenes::reply(state, intent) {
            return;
        }
        if job_action_scenes::reply(state, intent) {
            return;
        }
        if jobs_scenes::reply(state, intent) {
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
                let receipt = RegistrationReceipt::new(
                    "synthetic-deployment".into(),
                    "synthetic-node".into(),
                    "synthetic-registration".into(),
                    state
                        .registration
                        .selected_interface
                        .as_ref()
                        .map(|interface| interface.ip),
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
            Intent::UploadDataset { kind } | Intent::DownloadGithubDataset { kind } => {
                let dataset =
                    DatasetRef::new("synthetic-dataset".into(), *kind, "a".repeat(64), 1024)
                        .expect("synthetic dataset");
                state.maintenance.selected_dataset = Some(dataset.clone());
                state.maintenance.upload = Load::Ready(
                    serde_json::from_value(
                        serde_json::json!({"dataset":dataset,"expires_in_seconds":86400,"row_count":3}),
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

    fn setup_connected_embedded(state: &mut State) {
        readiness_scenes::embedded_status_checks(state);
        state.setup_gateway = true;
        state.setup_advanced_open = true;
    }

    fn setup_connected_distributed(state: &mut State) {
        readiness_scenes::mixed_status_checks(state);
        state.setup_gateway = true;
        state.setup_advanced_open = true;
    }

    fn inbox(state: &mut State) {
        registered(state);
        state.seed_inbox_drawer_fixture();
    }

    fn inbox_qa_off(state: &mut State, kind: BackendKind) {
        registered(state);
        state.seed_inbox_qa_drawer_fixture();
        state.backend_kind = kind;
        let can_manage = kind == BackendKind::Local;
        if can_manage {
            state.connection.origin = TextEdit::new("local://embedded");
        }
        state.seed_inbox_qa_fixture(QaSettings::new(false, QaPhase::Off, 1, 1, can_manage), None);
    }

    fn inbox_qa_unavailable(state: &mut State, reason: QaAvailabilityReason) {
        registered(state);
        state.seed_inbox_qa_drawer_fixture();
        state.backend_kind = BackendKind::Local;
        state.connection.origin = TextEdit::new("local://embedded");
        let mut settings = QaSettings::new(true, QaPhase::Unavailable, 2, 1, true);
        settings.availability_reason = Some(reason);
        state.seed_inbox_qa_fixture(settings, None);
    }

    fn inbox_qa_ready(state: &mut State) {
        registered(state);
        state.seed_inbox_qa_overflow_fixture();
        triage_connection(state, QaPhase::Ready);
    }

    fn inbox_qa_turn(
        state: &mut State,
        stage: QaTurnState,
        work: Option<QaTurnStage>,
        additional_finding: Option<QaFinding>,
    ) {
        registered(state);
        state.seed_inbox_qa_overflow_fixture();
        state.backend_kind = BackendKind::Local;
        state.connection.origin = TextEdit::new("local://embedded");
        let question = if additional_finding.is_some() {
            "Is actor search ready, and how much memory is in use?"
        } else if stage == QaTurnState::Answered {
            "Is actor search ready now, and what is a search index?"
        } else {
            "Is actor search ready?"
        };
        let mut turn = QaTurn::queued(
            "fixture-turn-1".into(),
            "fixture-request-1".into(),
            question.into(),
        );
        turn.state = stage;
        turn.stage = work;
        turn.cursor = 1;
        turn.accepted_at = Some(1_767_225_600);
        if stage == QaTurnState::Running && work.is_none() {
            turn.provisional_reply = Some("Let me check actor search readiness.".into());
        }
        if stage == QaTurnState::InsufficientContext {
            turn.limitation = Some(QaTurnLimitation::CapabilityUnavailable);
        }
        if matches!(stage, QaTurnState::Answered | QaTurnState::Canceled) {
            turn.completed_at = Some(1_767_225_602);
            turn.response_time_ms = Some(2_410);
            let answer = "Actor search is ready.";
            turn.findings.push(QaFinding::new(
                ChatSourceReference::new(ChatSourceKind::Readiness, "actor-search".into())
                    .expect("fixture search readiness source"),
                "Actor search".into(),
                answer.into(),
                0,
                answer.len() as u32,
                "2026-01-01T00:00:00Z".into(),
            ));
            turn.cursor = 3;
        }
        if stage == QaTurnState::Answered {
            state.selected_readiness = Some("actor_search".into());
            turn.reply = Some(if let Some(finding) = additional_finding {
                let reply = format!("Actor search is ready. {}", finding.text);
                turn.findings.push(finding);
                turn.cursor += 1;
                reply
            } else {
                "A search index organizes data so matching records can be found quickly.".into()
            });
            turn.context = Some(QaContextUsage::new(
                QaModelTarget::FlanT5Small.input_tokens(),
                448,
                1,
            ));
            let mut prior = QaTurn::queued(
                "fixture-turn-previous".into(),
                "fixture-request-previous".into(),
                "Is actor search ready?".into(),
            );
            prior.state = QaTurnState::Answered;
            prior.accepted_at = Some(1_767_225_480);
            prior.completed_at = Some(1_767_225_515);
            prior.response_time_ms = Some(35_000);
            prior.reply = Some("Actor search was ready.".into());
            prior.cursor = 2;
            state.seed_inbox_qa_history_fixture(vec![turn.clone(), prior]);
        }
        state.seed_inbox_qa_fixture(
            QaSettings::new(true, QaPhase::Ready, 2, 1, true),
            Some(turn),
        );
    }

    fn inbox_qa_sources(state: &mut State) {
        let text = "Workstation memory usage is 42%.";
        let metric = QaFinding::new(
            ChatSourceReference::new(ChatSourceKind::HostMetric, "memory".into())
                .expect("fixture memory source"),
            "Memory usage".into(),
            text.into(),
            0,
            text.len() as u32,
            "2026-01-01T00:00:01Z".into(),
        );
        inbox_qa_turn(state, QaTurnState::Answered, None, Some(metric));
    }

    fn triage_connection(state: &mut State, phase: QaPhase) {
        state.backend_kind = BackendKind::Local;
        state.connection.origin = TextEdit::new("local://embedded");
        if let Some(capabilities) = &mut state.connection.capabilities {
            capabilities.features.push(QA_TRIAGE_FEATURE.into());
        }
        state.seed_inbox_qa_fixture(QaSettings::new(true, phase, 2, 1, true), None);
    }

    fn jobs_assistant_explain(state: &mut State) {
        jobs_ready(state);
        triage_connection(state, QaPhase::Ready);
        state
            .connection
            .capabilities
            .as_mut()
            .expect("synthetic permissions")
            .permissions
            .push(Permission::RetryJob);
        let Load::Ready(detail) = &mut state.backend.detail else {
            panic!("Actor seed fixture must set job detail");
        };
        detail.job.title = "Actor seed".into();
        detail.job.state = "failed".into();
        detail.job.phase = "Failed".into();
        detail.job.error_summary = "Task lease expired after maximum worker recoveries".into();
        detail.job.can_retry = true;
        detail.job.activity_actions_available = true;
        detail.job.action_reason_code = "supported".into();
        detail.activity_actions_available = true;
        let row = detail.job.clone();
        jobs_scenes::install(state, vec![row]);
    }

    fn inbox_notice_actions(state: &mut State, phase: QaPhase) {
        readiness_scenes::observed(state);
        triage_connection(state, phase);
        if let Some(capabilities) = &mut state.connection.capabilities {
            capabilities.features.push(QA_MODEL_SETTINGS_FEATURE.into());
        }
        let mut settings = QaSettings::new(true, phase, 2, 1, true);
        settings.generator = Some(QaGeneratorSettings::new(
            QaGeneratorPreference::Automatic,
            Some(settings.model_target),
            phase,
            None,
        ));
        settings.decision = Some(QaDecisionSettings::new(
            QaDecisionPreference::RulesOnly,
            QaDecisionPreference::RulesOnly,
            QaPhase::Off,
            None,
        ));
        state.seed_inbox_qa_fixture(settings, None);
        state.fixture_show_triage_notice();
    }

    fn workstation_status(state: &mut State) {
        readiness_scenes::embedded_status_checks(state);
        state.host_metrics = Load::Ready(fleetworks_desktop::workstation_dashboard::HostMetrics {
            cpu_percent: Some(18.0),
            memory_percent: Some(0.0),
            disk_percent: None,
            ..Default::default()
        });
        jobs_scenes::install(state, Vec::new());
        state.seed_inbox_qa_fixture(QaSettings::new(true, QaPhase::Ready, 2, 1, true), None);
        state.workstation_status_checks_open = true;
    }

    fn workstation_settings(state: &mut State) {
        registered(state);
        state.fixture_show_workstation_settings();
    }

    fn assistant_settings(state: &mut State, qualified: bool) {
        registered(state);
        state.backend_kind = BackendKind::Local;
        state.connection.origin = TextEdit::new("local://embedded");
        let mut settings = QaSettings::new(true, QaPhase::Ready, 2, 1, true);
        if let Some(capabilities) = &mut state.connection.capabilities {
            capabilities.features.push(QA_MODEL_SETTINGS_FEATURE.into());
        }
        settings.generator = Some(QaGeneratorSettings::new(
            QaGeneratorPreference::Automatic,
            Some(settings.model_target),
            settings.phase,
            None,
        ));
        settings.decision = Some(QaDecisionSettings::new(
            QaDecisionPreference::Julia1Q8,
            QaDecisionPreference::RulesOnly,
            if qualified {
                QaPhase::Starting
            } else {
                QaPhase::Unavailable
            },
            (!qualified).then_some(QaDecisionAvailabilityReason::AwaitingQualification),
        ));
        state.seed_inbox_qa_fixture(settings, None);
        state.fixture_show_assistant_settings();
    }

    fn toolbar_telemetry_populated(state: &mut State) {
        workstation_status(state);
        state.host_metrics = Load::Ready(fleetworks_desktop::workstation_dashboard::HostMetrics {
            cpu_percent: Some(100.0),
            memory_percent: Some(89.0),
            disk_percent: Some(73.0),
            ..Default::default()
        });
    }

    enum ReviewAppearance {
        Matte,
        ReducedMotion,
    }

    fn review_appearance(state: &mut State, setup: fn(&mut State), appearance: ReviewAppearance) {
        setup(state);
        state.accessibility.fixture_override = Some(FixtureAccessibility {
            reduced_motion: matches!(appearance, ReviewAppearance::ReducedMotion),
            reduced_transparency: matches!(appearance, ReviewAppearance::Matte),
            ..FixtureAccessibility::default()
        });
    }

    fn tool_provider_keys_matte(state: &mut State) {
        review_appearance(
            state,
            redesign_scenes::tool_provider_keys,
            ReviewAppearance::Matte,
        );
    }

    fn tool_provider_keys_reduced_motion(state: &mut State) {
        review_appearance(
            state,
            redesign_scenes::tool_provider_keys,
            ReviewAppearance::ReducedMotion,
        );
    }

    pub fn run() {
        rune::run(AppSpec {
            init,
            scenes: &[
                ("backend-choice", State::fixture_show_backend_choice),
                ("backend-choice-opening", |state| {
                    state.startup = fleetworks_desktop::startup::Startup::cold_launch();
                }),
                ("backend-choice-matte", |state| {
                    review_appearance(
                        state,
                        State::fixture_show_backend_choice,
                        ReviewAppearance::Matte,
                    );
                }),
                ("backend-choice-reduced-motion", |state| {
                    state.fixture_show_backend_choice();
                    state.accessibility.fixture_override =
                        Some(fleetworks_desktop::accessibility::FixtureAccessibility {
                            reduced_motion: true,
                            ..Default::default()
                        });
                }),
                ("startup-embedded-registered", |state| {
                    startup_data_scenes::ready(state, false);
                }),
                ("startup-embedded-registered-reduced-motion", |state| {
                    startup_data_scenes::ready(state, true);
                }),
                ("startup-data-seed", startup_data_scenes::seed),
                ("startup-data-index", startup_data_scenes::index),
                (
                    "startup-data-index-queued",
                    startup_data_scenes::index_queued,
                ),
                // Stable scenario key used by the Linux capture workflow.
                ("startup-data-train", startup_data_scenes::index),
                (
                    "startup-health-checks-mixed",
                    startup_data_scenes::mixed_checks,
                ),
                (
                    "startup-data-checks-unavailable",
                    startup_data_scenes::checks_unavailable,
                ),
                ("redesign-status", redesign_scenes::status),
                ("redesign-seed", redesign_scenes::seed),
                ("redesign-seed-ready", redesign_scenes::seed_ready),
                (
                    "redesign-seed-invalid-header",
                    redesign_scenes::seed_invalid_header,
                ),
                ("redesign-seed-uploading", redesign_scenes::seed_uploading),
                (
                    "redesign-seed-confirmation",
                    redesign_scenes::seed_confirmation,
                ),
                ("redesign-brain", redesign_scenes::brain),
                ("redesign-tools", redesign_scenes::tools),
                ("redesign-models", redesign_scenes::models),
                ("redesign-jobs", redesign_scenes::jobs),
                ("redesign-setup", redesign_scenes::setup),
                ("pairing-idle", redesign_scenes::pairing_idle),
                ("pairing-waiting", redesign_scenes::pairing_waiting),
                (
                    "pairing-keychain-denied",
                    redesign_scenes::pairing_keychain_denied,
                ),
                ("pairing-denied", redesign_scenes::pairing_denied),
                ("pairing-expired", redesign_scenes::pairing_expired),
                (
                    "pairing-gateway-unavailable",
                    redesign_scenes::pairing_gateway_unavailable,
                ),
                ("pairing-connected", redesign_scenes::pairing_connected),
                ("logs-workstation-loaded", log_scenes::loaded),
                ("logs-export-partial", log_scenes::partial_export),
                ("logs-display-cap", log_scenes::display_cap),
                ("logs-cursor-changed", log_scenes::cursor_changed),
                ("logs-cohort-unavailable", log_scenes::cohort_unavailable),
                (
                    "connection-profiles-empty",
                    connection_profile_scenes::empty,
                ),
                (
                    "connection-profiles-saved",
                    connection_profile_scenes::saved,
                ),
                ("connection-profile-add", connection_profile_scenes::add),
                ("connection-profile-edit", connection_profile_scenes::edit),
                (
                    "connection-profile-tested",
                    connection_profile_scenes::tested,
                ),
                (
                    "connection-profile-auth-error",
                    connection_profile_scenes::auth_error,
                ),
                (
                    "connection-profile-save-conflict",
                    connection_profile_scenes::conflict,
                ),
                (
                    "connection-profile-switch-confirm",
                    connection_profile_scenes::dirty_switch,
                ),
                ("catalog", overhaul_scenes::catalog),
                ("catalog-opaque", overhaul_scenes::catalog_opaque),
                ("assistant-open", overhaul_scenes::assistant_open),
                ("assistant-opaque", overhaul_scenes::assistant_opaque),
                ("command-panel", overhaul_scenes::command_panel),
                (
                    "command-panel-opaque",
                    overhaul_scenes::command_panel_opaque,
                ),
                ("appearance-native", appearance_scenes::satin),
                ("appearance-matte", appearance_scenes::matte),
                (
                    "appearance-reduced-motion",
                    appearance_scenes::reduced_motion,
                ),
                ("appearance-material-fallback", appearance_scenes::fallback),
                ("setup-connected-embedded", setup_connected_embedded),
                ("setup-connected-distributed", setup_connected_distributed),
                ("inbox-drawer", inbox),
                ("inbox-qa-off-embedded", |state| {
                    inbox_qa_off(state, BackendKind::Local);
                }),
                ("inbox-qa-off-distributed", |state| {
                    inbox_qa_off(state, BackendKind::Remote);
                }),
                ("inbox-qa-ready", inbox_qa_ready),
                ("inbox-qa-pending", |state| {
                    inbox_qa_turn(state, QaTurnState::Running, None, None);
                }),
                ("inbox-qa-working", |state| {
                    inbox_qa_turn(
                        state,
                        QaTurnState::Running,
                        Some(QaTurnStage::Working),
                        None,
                    );
                }),
                ("inbox-qa-thinking", |state| {
                    inbox_qa_turn(
                        state,
                        QaTurnState::Running,
                        Some(QaTurnStage::Thinking),
                        None,
                    );
                }),
                ("inbox-qa-writing", |state| {
                    inbox_qa_turn(
                        state,
                        QaTurnState::Running,
                        Some(QaTurnStage::Writing),
                        None,
                    );
                }),
                ("inbox-qa-history-unavailable", |state| {
                    inbox_qa_turn(state, QaTurnState::InsufficientContext, None, None);
                }),
                ("inbox-qa-answered", |state| {
                    inbox_qa_turn(state, QaTurnState::Answered, None, None);
                }),
                ("inbox-qa-sources", inbox_qa_sources),
                ("inbox-qa-stopped", |state| {
                    inbox_qa_turn(state, QaTurnState::Canceled, None, None);
                }),
                ("inbox-qa-pack-missing", |state| {
                    inbox_qa_unavailable(state, QaAvailabilityReason::PackMissing);
                }),
                ("inbox-qa-pack-invalid", |state| {
                    inbox_qa_unavailable(state, QaAvailabilityReason::PackInvalid);
                }),
                ("inbox-qa-sandbox-unsupported", |state| {
                    inbox_qa_unavailable(state, QaAvailabilityReason::SandboxUnsupported);
                }),
                ("inbox-qa-worker-start-failed", |state| {
                    inbox_qa_unavailable(state, QaAvailabilityReason::WorkerStartFailed);
                }),
                ("gateway-qa-sandbox-unsupported", |state| {
                    inbox_qa_unavailable(state, QaAvailabilityReason::SandboxUnsupported);
                    state.fixture_show_gateway_qa_details();
                }),
                ("workstation-status-checks-mixed", workstation_status),
                ("workstation-status-compact", |state| {
                    workstation_status(state);
                    state.workstation_status_checks_open = false;
                }),
                ("workstation-settings", workstation_settings),
                ("workstation-settings-assistant", |state| {
                    assistant_settings(state, false);
                }),
                ("workstation-settings-assistant-provisioning", |state| {
                    assistant_settings(state, true);
                }),
                ("toolbar-telemetry-populated", toolbar_telemetry_populated),
                ("tools-catalog-ready-matte", |state| {
                    review_appearance(state, tools_ready, ReviewAppearance::Matte);
                }),
                ("models-ready-matte", |state| {
                    review_appearance(state, models_ready, ReviewAppearance::Matte);
                }),
                ("workstation-status-checks-mixed-matte", |state| {
                    review_appearance(state, workstation_status, ReviewAppearance::Matte);
                }),
                ("workstation-status-checks-mixed-reduced-motion", |state| {
                    review_appearance(state, workstation_status, ReviewAppearance::ReducedMotion);
                }),
                ("inbox-drawer-matte", |state| {
                    review_appearance(state, inbox, ReviewAppearance::Matte);
                }),
                ("inbox-drawer-reduced-motion", |state| {
                    review_appearance(state, inbox, ReviewAppearance::ReducedMotion);
                }),
                ("command-panel-reduced-motion", |state| {
                    review_appearance(
                        state,
                        overhaul_scenes::command_panel,
                        ReviewAppearance::ReducedMotion,
                    );
                }),
                ("setup-connected-embedded-matte", |state| {
                    review_appearance(state, setup_connected_embedded, ReviewAppearance::Matte);
                }),
                ("setup-connected-embedded-reduced-motion", |state| {
                    review_appearance(
                        state,
                        setup_connected_embedded,
                        ReviewAppearance::ReducedMotion,
                    );
                }),
                ("setup-connected-distributed-matte", |state| {
                    review_appearance(state, setup_connected_distributed, ReviewAppearance::Matte);
                }),
                ("setup-connected-distributed-reduced-motion", |state| {
                    review_appearance(
                        state,
                        setup_connected_distributed,
                        ReviewAppearance::ReducedMotion,
                    );
                }),
                ("tool-provider-keys-matte", tool_provider_keys_matte),
                (
                    "tool-provider-keys-reduced-motion",
                    tool_provider_keys_reduced_motion,
                ),
                ("atlas-provider-keys-matte", tool_provider_keys_matte),
                (
                    "atlas-provider-keys-reduced-motion",
                    tool_provider_keys_reduced_motion,
                ),
                ("workstation-settings-matte", |state| {
                    review_appearance(state, workstation_settings, ReviewAppearance::Matte);
                }),
                ("workstation-settings-reduced-motion", |state| {
                    review_appearance(state, workstation_settings, ReviewAppearance::ReducedMotion);
                }),
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
                (
                    "workstation-bert-verified",
                    readiness_scenes::inference_verified,
                ),
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
                // Existing workflow selectors capture supported native replacements.
                ("atlas-offline", |state| state.page = Page::Workstation),
                ("atlas-articles-ready", redesign_scenes::status),
                (
                    "atlas-prerequisites",
                    readiness_scenes::inference_unverified,
                ),
                ("atlas-provider-keys", redesign_scenes::tool_provider_keys),
                ("tool-provider-keys", redesign_scenes::tool_provider_keys),
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
                ("jobs-assistant-explain", jobs_assistant_explain),
                ("inbox-notice-actions-starting", |state| {
                    inbox_notice_actions(state, QaPhase::Starting);
                }),
                ("inbox-notice-actions", |state| {
                    inbox_notice_actions(state, QaPhase::Ready);
                }),
                ("jobs-grouped-list", jobs_scenes::grouped_list),
                ("jobs-filters-ready", jobs_scenes::filters_ready),
                ("jobs-page-two", jobs_scenes::page_two),
                ("jobs-no-matches", jobs_scenes::no_matches),
                ("jobs-page-expired", jobs_scenes::expired),
                ("jobs-permission-required", jobs_scenes::permission_required),
                ("jobs-query-loading", jobs_scenes::loading),
                ("jobs-query-error", jobs_scenes::error),
                ("jobs-status-only", jobs_scenes::status_only),
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
                    "jobs-legacy-classifier-result",
                    lifecycle_scenes::legacy_classifier_result,
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

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn main() {
    native::run();
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn main() -> anyhow::Result<()> {
    anyhow::bail!(
        "The native fixture requires a macOS or Linux Rune renderer; this host cannot provide native fixture evidence."
    )
}
