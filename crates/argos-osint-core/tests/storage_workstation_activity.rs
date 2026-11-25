//! Requester membership must filter every activity read before pagination.
#![cfg(feature = "surreal")]

mod support;

use argos_osint_core::jobs_view::JobFilter;
use argos_osint_core::storage::bootstrap;
use argos_osint_core::storage::connect_registry;
use argos_osint_core::storage::connect_registry::RegistryAuthority;
use argos_osint_core::storage::inbox_qa as qa;
use argos_osint_core::storage::job_context::InteractiveAdmission;
use argos_osint_core::storage::job_dispatch;
use argos_osint_core::storage::jobs;
use argos_osint_core::storage::jobs::NewJob;
use argos_osint_core::storage::jobs::NewTask;
use argos_osint_core::storage::jobs_view as view;
use argos_osint_core::storage::repo::Repository;
use argos_osint_core::storage::tx::TxError;
use chrono::Utc;
use fleetworks_contracts::chat::ChatSourceKind;
use fleetworks_contracts::chat::ChatSourceReference;
use fleetworks_contracts::chat::QaDecisionPreference;
use fleetworks_contracts::chat::QaGeneratorPreference;
use fleetworks_contracts::chat::QaTurnRequest;
use fleetworks_contracts::chat::QaTurnState;
use fleetworks_contracts::connect::RegistrationEnsureRequest;
use fleetworks_contracts::context::ActivityScope;
use fleetworks_contracts::context::JobOrigin;
use fleetworks_contracts::context::MutationContext;
use fleetworks_contracts::context::OriginSource;
use fleetworks_contracts::context::RegisteredContext;
use fleetworks_contracts::qa_worker::QaDialogueKind;
use fleetworks_contracts::qa_worker::QaPassage;
use fleetworks_contracts::qa_worker::QaWorkerDialogue;
use fleetworks_contracts::qa_worker::QaWorkerRequest;
use support::Server;
use surrealdb::engine::any::Any;
use surrealdb::types::RecordId;
use surrealdb::Surreal;

const QA_WORKER: &str = "qa-model-fixture";
const QA_PLACEMENT: &str = "qa-placement-fixture";

fn authority() -> RegistryAuthority {
    RegistryAuthority::new("scope-a", "fixture-native", "fixture-access")
        .expect("fixture authority")
}

async fn register(repo: &Repository, node: &str, child: &str) -> RegisteredContext {
    bootstrap::ensure_network_identity(repo.db())
        .await
        .expect("explicit fixture bootstrap identity");
    connect_registry::initialize_control(repo.db())
        .await
        .expect("explicit fixture control");
    let ip = if node == "node-a" {
        "192.0.2.42"
    } else {
        "198.51.100.42"
    };
    let receipt = connect_registry::ensure(
        repo,
        &authority(),
        &RegistrationEnsureRequest::new(
            format!("ensure-{node}-{child}"),
            Some(ip.parse().expect("synthetic IP")),
            format!("installation-{node}"),
        ),
    )
    .await
    .expect("actual fixture registration")
    .receipt;
    connect_registry::registration_context(
        repo,
        &authority(),
        &receipt.registration_id,
        receipt.revision,
    )
    .await
    .expect("actual current registration context")
}

fn scope(context: &RegisteredContext, child: bool) -> ActivityScope {
    let _ = child;
    ActivityScope::for_workstation(context)
}

async fn job(db: &Surreal<Any>, id: &str, parent: Option<&str>, kind: &str) {
    jobs::create_job(
        db,
        &NewJob {
            id,
            parent,
            kind,
            operation: "summaries",
            created_at: Utc::now(),
        },
        &[],
    )
    .await
    .expect("fixture job");
    db.query("UPDATE $job SET state='queued', app='intel', title=$title, error_summary=$error, provider='private-provider', result_ref='private-result' RETURN NONE")
        .bind(("job", surrealdb::types::RecordId::new("job", id.to_owned())))
        .bind(("title", format!("private-title-{id}"))).bind(("error", format!("private-error-{id}")))
        .await.expect("fixture metadata").check().expect("fixture metadata statement");
}

async fn associate(
    repo: &Repository,
    context: &RegisteredContext,
    id: &str,
    key: &str,
    kind: &str,
) {
    let origin = JobOrigin::new(
        context,
        OriginSource::Native,
        key.into(),
        Utc::now().to_rfc3339(),
        1,
        Vec::new(),
    );
    let admission = InteractiveAdmission::native(authority(), context.clone(), origin)
        .expect("trusted fixture adapter");
    job_dispatch::enable_pool(repo.db(), "activity_fixture")
        .await
        .expect("explicit fixture consumer");
    let task_id = format!("{id}:execute");
    job_dispatch::submit_interactive(
        repo.db(),
        &NewJob {
            id,
            parent: None,
            kind,
            operation: "summaries",
            created_at: Utc::now(),
        },
        &NewTask {
            id: &task_id,
            job: id,
            pool: "activity_fixture",
            operation: "summaries",
            input_hash: "fixture-input",
            priority: 0,
            deps: Vec::new(),
            created_at: Utc::now(),
        },
        &admission,
    )
    .await
    .expect("actual proof-fenced job/task/outbox admission");
    repo.db().query("UPDATE $job SET app='intel', title=$title, error_summary=$error, provider='private-provider', result_ref='private-result' RETURN NONE")
        .bind(("job", surrealdb::types::RecordId::new("job", id.to_owned())))
        .bind(("title", format!("private-title-{id}"))).bind(("error", format!("private-error-{id}")))
        .await.expect("fixture metadata").check().expect("fixture metadata statement");
}

#[tokio::test]
async fn node_child_deployment_and_causal_membership_filter_before_limits_and_facets() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "workstation_activity_scope").await;
    let db = repo.db();
    let first = register(&repo, "node-a", "research").await;
    let second = register(&repo, "node-a", "analysis").await;
    assert_eq!(first.workstation_id, second.workstation_id);
    assert_eq!(first.registration_id, second.registration_id);
    let foreign = register(&repo, "node-b", "research").await;
    let ordering_epoch = Utc::now();
    for (id, context) in [
        ("owned-a", &first),
        ("owned-b", &second),
        ("other-node", &foreign),
    ] {
        associate(&repo, context, id, id, "task").await;
    }
    job(db, "other-deployment", None, "task").await;
    // Adversarial foreign-deployment metadata must not grant membership.
    db.query("CREATE job_origin:other_deployment SET job_id='other-deployment', root_id='other-deployment', scope_id='scope-a', deployment_id='foreign-deployment', workstation_id=$node, workspace_id=$child, origin_json='{}'; CREATE job_request:other_deployment SET request_key='foreign-deployment', operation='summaries', job_id='other-deployment', scope_id='scope-a', deployment_id='foreign-deployment', registration_id=$registration, workstation_id=$node, workspace_id=$child, requested_at=$now, origin_json='{}';")
        .bind(("node",first.workstation_id.clone())).bind(("child",String::from("legacy-child")))
        .bind(("registration",first.registration_id.clone())).bind(("now",Utc::now().to_rfc3339()))
        .await.expect("adversarial foreign fixture").check().expect("foreign fixture statement");
    job(db, "legacy", None, "task").await;
    associate(&repo, &first, "system", "service-request", "service").await;
    // The production child path copies origin under the real registry fence.
    job(db, "causal-child", Some("owned-a"), "task").await;
    job(db, "parent-link-only", None, "task").await;
    db.query("UPDATE $child SET parent=$parent,root=$parent RETURN NONE")
        .bind((
            "child",
            surrealdb::types::RecordId::new("job", "parent-link-only"),
        ))
        .bind(("parent", surrealdb::types::RecordId::new("job", "owned-a")))
        .await
        .expect("adversarial parent-only fixture")
        .check()
        .expect("parent fixture statement");
    db.query("FOR $i IN 0..205 { LET $job = type::record('job', string::concat('foreign-', <string>$i)); CREATE $job SET root=$job, kind='task', operation='summaries', state='running', app='foreign-only', created_at=time::now(), updated_at=<datetime>$foreign_updated; };")
        .bind(("foreign_updated", (ordering_epoch + chrono::Duration::seconds(1)).to_rfc3339()))
        .await.expect("crowded foreign jobs").check().expect("crowded foreign statements");
    // Preserve genuine admission/origins; pin only mutable ordering metadata.
    // Tied owned rows use ascending key, while the newer foreign crowd must be
    // excluded before either the large or one-row LIMIT is applied.
    db.query("UPDATE job SET updated_at=<datetime>$owned_updated WHERE id IN [$first,$second] RETURN NONE")
        .bind(("owned_updated", ordering_epoch.to_rfc3339()))
        .bind(("first", surrealdb::types::RecordId::new("job", "owned-a")))
        .bind(("second", surrealdb::types::RecordId::new("job", "owned-b")))
        .await.expect("controlled tied ordering after real admission").check().expect("ordering statement");
    let node = scope(&first, false);
    let rows = view::list_activity_jobs(db, "scope-a", &node, &JobFilter::default(), 200)
        .await
        .expect("node activity");
    let ids: std::collections::BTreeSet<_> = rows.iter().map(|row| row.job.id.as_str()).collect();
    assert_eq!(ids, ["owned-a", "owned-b"].into_iter().collect());
    let small_page = view::list_activity_jobs(db, "scope-a", &node, &JobFilter::default(), 1)
        .await
        .expect("small page");
    assert_eq!(small_page.len(), 1);
    assert_eq!(
        small_page[0].job.id, "owned-a",
        "ascending key breaks equal rank/update ties after membership filtering"
    );
    let child = scope(&first, true);
    assert_eq!(
        view::list_activity_jobs(db, "scope-a", &child, &JobFilter::default(), 100)
            .await
            .expect("child page")[0]
            .job
            .id,
        "owned-a"
    );
    assert_eq!(
        view::activity_job_counts(db, "scope-a", &node)
            .await
            .expect("scoped counts")
            .queued,
        2
    );
    assert_eq!(
        view::activity_job_apps(db, "scope-a", &node)
            .await
            .expect("scoped app facets"),
        vec!["intel"]
    );
    assert!(
        view::get_activity_job(db, "scope-a", &child, "causal-child")
            .await
            .expect("causal child")
            .is_some()
    );
    for id in [
        "other-node",
        "other-deployment",
        "legacy",
        "system",
        "parent-link-only",
    ] {
        assert!(
            view::get_activity_job(db, "scope-a", &child, id)
                .await
                .expect("guessed ID check")
                .is_none(),
            "unauthorized guessed ID {id}"
        );
    }
    assert!(
        view::list_activity_jobs(db, "different-scope", &node, &JobFilter::default(), 100)
            .await
            .expect("other authority")
            .is_empty()
    );
}

#[tokio::test]
async fn shared_reuse_preserves_creator_but_private_fields_and_search_stay_unavailable() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "workstation_activity_shared").await;
    let db = repo.db();
    let first = register(&repo, "node-a", "research").await;
    let other = register(&repo, "node-b", "analysis").await;
    associate(&repo, &first, "shared", "original", "task").await;
    let first_scope = scope(&first, true);
    let before = view::get_activity_job(db, "scope-a", &first_scope, "shared")
        .await
        .expect("original job")
        .expect("member");
    assert!(before.private_details_available);
    assert_eq!(before.job.title, "private-title-shared");

    let actor = qa::Actor::new(
        "scope-a".into(),
        "fixture-native".into(),
        "fixture-access".into(),
        first.clone(),
    );
    let setting = qa::set(&repo, &actor, 0, true).await.expect("enable QA");
    let source = ChatSourceReference::new(ChatSourceKind::Job, "shared".into()).unwrap();
    let request = QaTurnRequest::new(
        "private-job-answer".into(),
        MutationContext::new(&first),
        "What happened?".into(),
        Some(source.clone()),
    )
    .unwrap();
    let id = qa::accepted_turn_id(&actor, &request, setting.history_generation).unwrap();
    let at = Utc::now();
    let evidence = qa::FrozenEvidence {
        answer_mode: qa::AnswerMode::Grounded,
        notice: None,
        conversation: None,
        request: QaWorkerRequest::new(
            id.clone(),
            id,
            request.question.clone(),
            vec![QaPassage::new(
                "passage-0".into(),
                "Selected job is queued.".into(),
            )],
        ),
        sources: vec![qa::Source {
            opaque_id: "passage-0".into(),
            reference: source.clone(),
            label: "Selected job".into(),
            observed_at: at.to_rfc3339(),
            measurement: None,
            whole_passage: false,
        }],
        sampled_at_ms: at.timestamp_millis(),
    };
    let metadata = || qa::AdmissionMetadata {
        subject: Some(source.clone()),
        observation: None,
        related_activity: None,
    };
    let answer = qa::admit(
        &repo,
        &actor,
        &request,
        evidence.clone(),
        metadata(),
        setting.generation,
    )
    .await
    .expect("private job answer");
    assert_eq!(answer.state, QaTurnState::Answered);
    assert_eq!(
        qa::replay(&repo, &actor, &request).await.unwrap(),
        Some(answer)
    );

    associate(&repo, &other, "shared", "reuse-other-node", "task").await;
    assert!(matches!(
        qa::replay(&repo, &actor, &request).await,
        Err(TxError::Abort(reason)) if reason == "inbox_qa:missing"
    ));
    let mut pending = request;
    pending.request_id = "private-job-race".into();
    assert!(matches!(
        qa::admit(&repo, &actor, &pending, evidence, metadata(), setting.generation).await,
        Err(TxError::Abort(reason)) if reason == "inbox_qa:missing"
    ));
    assert!(qa::latest(&repo, &actor).await.unwrap().is_none());
    assert!(qa::last_answered_prompt(&repo, &actor)
        .await
        .unwrap()
        .is_none());

    for selected in [first_scope, scope(&other, true)] {
        let row = view::get_activity_job(db, "scope-a", &selected, "shared")
            .await
            .expect("shared job")
            .expect("genuine requester");
        assert!(
            row.status_only && !row.private_details_available && !row.activity_actions_available
        );
        assert_eq!(row.job.id, "shared");
        assert_eq!(row.job.state, "queued");
        assert!(!row.job.cancellable);
        let json = serde_json::to_string(&row).expect("row wire serialization");
        for private in [
            "private-title",
            "private-error",
            "private-provider",
            "private-result",
        ] {
            assert!(
                !json.contains(private),
                "private value escaped status projection"
            );
        }
        let filter = JobFilter {
            text: "private-title-shared".into(),
            ..Default::default()
        };
        assert!(
            view::list_activity_jobs(db, "scope-a", &selected, &filter, 100)
                .await
                .expect("private search oracle check")
                .is_empty()
        );
    }
    let mut result=db.query("RETURN job_origin:shared.workstation_id; SELECT VALUE workstation_id FROM job_request WHERE job_id='shared';")
        .await.expect("creator audit").check().expect("creator audit statements");
    assert_eq!(
        result
            .take::<Option<String>>(0)
            .expect("saved creator")
            .as_deref(),
        Some(first.workstation_id.as_str())
    );
    let mut requesters = result.take::<Vec<String>>(1).expect("request associations");
    let mut expected = vec![first.workstation_id.clone(), other.workstation_id.clone()];
    requesters.sort();
    expected.sort();
    assert_eq!(requesters, expected);
}

#[tokio::test]
async fn qa_generator_replacement_fences_leased_output_and_leaves_decision_independent() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server
        .repo("argos", "workstation_assistant_preferences")
        .await;
    let context = register(&repo, "node-a", "research").await;
    let actor = qa::Actor::new(
        "scope-a".into(),
        "fixture-native".into(),
        "fixture-access".into(),
        context.clone(),
    );
    let db = repo.db();
    db.query("CREATE inbox_qa_settings:current SET enabled=false, revision=4, generation=3, epoch='', model_target='flan_t5_base' RETURN NONE")
        .await.unwrap().check().unwrap();
    let migrated = qa::setting(&repo).await.unwrap();
    assert_eq!(
        migrated.generator_preference,
        QaGeneratorPreference::Automatic
    );
    assert_eq!(
        migrated.decision_preference,
        QaDecisionPreference::RulesOnly
    );
    let enabled = qa::set(&repo, &actor, migrated.revision, true)
        .await
        .unwrap();

    let (request, lease) = start_qa_fixture(
        &repo,
        &actor,
        &context,
        enabled,
        "lease-before-model-switch",
    )
    .await;

    let decision = qa::update(
        &repo,
        &actor,
        enabled.revision,
        qa::SettingUpdate {
            decision_preference: Some(QaDecisionPreference::Julia1Q8),
            ..qa::SettingUpdate::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(decision.generation, enabled.generation);
    assert!(
        qa::beat(&repo, QA_PLACEMENT, QA_WORKER, enabled.generation, true)
            .await
            .unwrap()
    );
    let running = qa::replay(&repo, &actor, &request).await.unwrap().unwrap();
    assert_eq!(running.state, QaTurnState::Running);

    let generator = qa::update(
        &repo,
        &actor,
        decision.revision,
        qa::SettingUpdate {
            generator_preference: Some(QaGeneratorPreference::FlanT5Small),
            ..qa::SettingUpdate::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(generator.decision_generation, decision.decision_generation);
    assert!(
        !qa::beat(&repo, QA_PLACEMENT, QA_WORKER, enabled.generation, true)
            .await
            .unwrap()
    );
    assert!(!qa::publish(&repo, &lease, QaTurnState::Failed, None)
        .await
        .unwrap());
    let canceled = qa::replay(&repo, &actor, &request).await.unwrap().unwrap();
    assert_eq!(canceled.turn_id, running.turn_id);
    assert_eq!(canceled.state, QaTurnState::Canceled);
    assert!(canceled.findings.is_empty());
}

async fn start_qa_fixture(
    repo: &Repository,
    actor: &qa::Actor,
    context: &RegisteredContext,
    enabled: qa::Setting,
    request_id: &str,
) -> (QaTurnRequest, Box<job_dispatch::DeliveryLease>) {
    let db = repo.db();
    assert_eq!(
        qa::claim_replica(repo, QA_PLACEMENT, QA_WORKER, enabled.generation)
            .await
            .unwrap(),
        qa::ReplicaClaim::Warm,
    );
    assert!(
        qa::beat(repo, QA_PLACEMENT, QA_WORKER, enabled.generation, true)
            .await
            .unwrap()
    );
    job_dispatch::enable_pool(db, qa::POOL).await.unwrap();
    let request = QaTurnRequest::new(
        request_id.into(),
        MutationContext::new(context),
        "Which backend checks are ready?".into(),
        None,
    )
    .unwrap();
    let id = qa::accepted_turn_id(actor, &request, enabled.history_generation).unwrap();
    let at = Utc::now();
    let facts = [
        ("gateway", "Gateway", "Gateway is ready."),
        ("catalog", "Actor catalog", "Actor catalog is ready."),
    ];
    let mut worker_request = QaWorkerRequest::new(
        id.clone(),
        id,
        request.question.clone(),
        facts
            .iter()
            .map(|(id, _, text)| QaPassage::new((*id).into(), (*text).into()))
            .collect(),
    );
    worker_request.dialogue = Some(QaWorkerDialogue {
        kind: QaDialogueKind::Operational,
        history: Vec::new(),
    });
    let evidence = qa::FrozenEvidence {
        answer_mode: qa::AnswerMode::Model,
        notice: None,
        conversation: None,
        request: worker_request,
        sources: facts
            .iter()
            .map(|(id, label, _)| qa::Source {
                opaque_id: (*id).into(),
                reference: ChatSourceReference::new(ChatSourceKind::Readiness, (*id).into())
                    .unwrap(),
                label: (*label).into(),
                observed_at: at.to_rfc3339(),
                measurement: None,
                whole_passage: false,
            })
            .collect(),
        sampled_at_ms: at.timestamp_millis(),
    };
    let queued = qa::admit(
        repo,
        actor,
        &request,
        evidence,
        qa::AdmissionMetadata {
            subject: None,
            observation: None,
            related_activity: None,
        },
        enabled.generation,
    )
    .await
    .unwrap();
    assert_eq!(queued.state, QaTurnState::Queued);
    let claim = job_dispatch::claim_ready(db, qa::POOL, "qa-fixture-relay", Utc::now())
        .await
        .unwrap()
        .expect("model delivery");
    let job_dispatch::DeliveryClaim::Claimed(lease) = job_dispatch::claim_delivery(
        db,
        &claim.envelope,
        QA_WORKER,
        30,
        Utc::now(),
        "qa-fixture-claim",
    )
    .await
    .unwrap() else {
        panic!("model delivery must be leased");
    };
    assert!(qa::start(repo, &lease).await.unwrap().is_some());
    (request, lease)
}

#[tokio::test]
async fn qa_fallback_seals_all_findings_atomically_under_the_live_lease() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "workstation_assistant_fallback").await;
    let context = register(&repo, "node-a", "research").await;
    let actor = qa::Actor::new(
        "scope-a".into(),
        "fixture-native".into(),
        "fixture-access".into(),
        context.clone(),
    );
    let enabled = qa::set(
        &repo,
        &actor,
        qa::setting(&repo).await.unwrap().revision,
        true,
    )
    .await
    .unwrap();
    let (_, lease) = start_qa_fixture(&repo, &actor, &context, enabled, "fallback-batch").await;
    let evidence = qa::start(&repo, &lease).await.unwrap().unwrap();
    let findings: Vec<_> = evidence
        .request
        .passages
        .iter()
        .zip(&evidence.sources)
        .map(|(passage, source)| {
            source.finding(
                passage.text.clone(),
                0,
                u32::try_from(passage.text.chars().count()).unwrap(),
            )
        })
        .collect();
    let before = qa::latest(&repo, &actor).await.unwrap().unwrap().0;
    let duplicate = [findings[0].clone(), findings[0].clone()];
    assert!(matches!(
        qa::publish_findings_and_complete(
            &repo, &lease, QaTurnState::Answered, &duplicate, qa::ModelResult::default(),
        )
        .await,
        Err(TxError::Abort(reason)) if reason == "inbox_qa:invalid_part"
    ));
    let (running, _) = qa::latest(&repo, &actor).await.unwrap().unwrap();
    assert_eq!(running.state, QaTurnState::Running);
    assert_eq!(running.cursor, before.cursor);
    assert!(running.findings.is_empty());

    assert!(qa::publish_findings_and_complete(
        &repo,
        &lease,
        QaTurnState::Answered,
        &findings,
        qa::ModelResult::default(),
    )
    .await
    .unwrap());
    let (answered, events) = qa::latest(&repo, &actor).await.unwrap().unwrap();
    assert_eq!(answered.state, QaTurnState::Answered);
    assert_eq!(answered.findings, findings);
    assert_eq!(answered.cursor, before.cursor + 3);
    assert_eq!(events.last().unwrap().state, QaTurnState::Answered);
    assert!(!qa::publish_findings_and_complete(
        &repo,
        &lease,
        QaTurnState::Answered,
        &findings,
        qa::ModelResult::default(),
    )
    .await
    .unwrap());
}

#[tokio::test]
async fn qa_off_serializes_with_worker_and_health_writes_and_retires_their_generation() {
    let Some(server) = Server::start() else {
        return;
    };
    let repo = server.repo("argos", "workstation_assistant_off").await;
    let context = register(&repo, "node-a", "research").await;
    let actor = qa::Actor::new(
        "scope-a".into(),
        "fixture-native".into(),
        "fixture-access".into(),
        context.clone(),
    );
    let enabled = qa::update(
        &repo,
        &actor,
        qa::setting(&repo).await.unwrap().revision,
        qa::SettingUpdate {
            enabled: Some(true),
            generator_preference: Some(QaGeneratorPreference::FlanT5Small),
            decision_preference: Some(QaDecisionPreference::Julia1Q8),
        },
    )
    .await
    .unwrap();
    let (request, lease) =
        start_qa_fixture(&repo, &actor, &context, enabled, "lease-before-off").await;
    assert_eq!(
        qa::latest(&repo, &actor).await.unwrap().unwrap().0.state,
        QaTurnState::Running,
    );

    const PROBE: &str = "qa-health-fixture";
    assert!(matches!(
        qa::claim_health(&repo, PROBE, enabled.generation, 0, true)
            .await
            .unwrap(),
        qa::HealthClaim::Probe(_),
    ));
    let sample = qa::HealthSample {
        sampled_at_ms: Utc::now().timestamp_millis(),
        facts: Vec::new(),
    };
    let worker_repo = repo.clone();
    let probe_repo = repo.clone();
    let reader_repo = repo.clone();
    // The transcript lookup also writes the registration guard.
    let (renewed, sampled, transcript, off) = tokio::join!(
        biased;
        qa::beat(&worker_repo, QA_PLACEMENT, QA_WORKER, enabled.generation, true),
        qa::put_health(&probe_repo, PROBE, enabled.generation, &sample),
        qa::latest(&reader_repo, &actor),
        qa::set(&repo, &actor, enabled.revision, false),
    );
    assert!(renewed.expect("worker heartbeat before Off"));
    assert!(sampled.expect("health publication before Off"));
    assert_eq!(
        transcript
            .expect("transcript lookup before Off")
            .unwrap()
            .0
            .state,
        QaTurnState::Running,
    );
    let off = off.expect("one Off request commits despite overlapping QA writes");
    assert_eq!(qa::setting(&repo).await.unwrap(), off);
    assert!(!off.enabled);
    assert_eq!(off.revision, enabled.revision + 1);
    assert_eq!(off.generator_preference, enabled.generator_preference);
    assert_eq!(off.decision_preference, enabled.decision_preference);

    assert!(!qa::beat(
        &worker_repo,
        QA_PLACEMENT,
        QA_WORKER,
        enabled.generation,
        true
    )
    .await
    .unwrap());
    assert!(
        !qa::put_health(&probe_repo, PROBE, enabled.generation, &sample)
            .await
            .unwrap()
    );
    assert!(
        !qa::publish(&worker_repo, &lease, QaTurnState::Failed, None)
            .await
            .unwrap()
    );
    assert!(matches!(
        qa::replay(&repo, &actor, &request).await,
        Err(TxError::Abort(reason)) if reason == "inbox_qa:off"
    ));
    assert!(qa::latest(&repo, &actor).await.unwrap().is_none());
    let mut stored = repo
        .db()
        .query(
            "RETURN record::exists($job) OR record::exists($task) OR record::exists($turn)
                OR record::exists(inbox_qa_health:current);
         SELECT VALUE true FROM attempt WHERE root=$job LIMIT 1;
         SELECT VALUE true FROM inbox_qa_latest LIMIT 1;",
        )
        .bind(("job", RecordId::new("job", lease.task.job_id.clone())))
        .bind(("task", RecordId::new("task", lease.task.id.clone())))
        .bind((
            "turn",
            RecordId::new("inbox_qa_exchange", lease.task.job_id.clone()),
        ))
        .await
        .unwrap()
        .check()
        .unwrap();
    assert_eq!(stored.take::<Option<bool>>(0).unwrap(), Some(false));
    assert!(stored.take::<Vec<bool>>(1).unwrap().is_empty());
    assert!(stored.take::<Vec<bool>>(2).unwrap().is_empty());

    assert!(qa::replica_stopping(&repo).await.unwrap());
    qa::forget_replica(&worker_repo, QA_PLACEMENT, QA_WORKER)
        .await
        .unwrap();
    assert!(!qa::replica_stopping(&repo).await.unwrap());
}
