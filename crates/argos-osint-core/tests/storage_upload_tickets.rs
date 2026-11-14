//! Shared ticket quota, atomic claims and restart/recovery contracts.
#![cfg(feature = "surreal")]
mod support;

use argos_osint_core::storage::repo::Repository;
use argos_osint_core::storage::tx::TxError;
use argos_osint_core::storage::upload_ticket;
use argos_osint_core::storage::upload_ticket::UploadTicketBinding;
use fleetworks_contracts::connect::RegistrationReceipt;
use fleetworks_contracts::context::CredentialPurpose;
use fleetworks_contracts::context::RegisteredContext;
use fleetworks_contracts::dataset_upload::DatasetUploadRequest;
use fleetworks_contracts::readiness::DatasetKind;
use surrealdb::types::RecordId;

fn context(registration: &str) -> RegisteredContext {
    RegisteredContext::new(
        RegistrationReceipt::new(
            "deployment".into(),
            format!("workstation-{registration}"),
            registration.into(),
            None,
            1,
        ),
        1,
        1,
        "configuration-epoch".into(),
        "access-epoch".into(),
    )
    .expect("trusted synthetic registered context")
}
fn binding(credential: &str, registered: &RegisteredContext) -> UploadTicketBinding {
    UploadTicketBinding::new(
        "scope",
        credential,
        "access-epoch",
        CredentialPurpose::Native,
        registered,
    )
    .expect("trusted authority")
}
fn request() -> DatasetUploadRequest {
    DatasetUploadRequest::new(DatasetKind::ActorCatalog, "a".repeat(64), 128)
        .expect("exact upload metadata")
}
fn id(path: &str) -> &str {
    path.rsplit('/')
        .next()
        .expect("server-issued relative ticket")
}
fn refused<T>(result: &Result<T, TxError>, code: &str) -> bool {
    matches!(result,Err(TxError::Abort(actual)) if actual == &format!("upload_ticket:{code}"))
}
async fn expire_one(repo: &Repository, ticket: &str) {
    repo.db()
        .query("UPDATE $row SET expires_at=time::now()-1s RETURN NONE")
        .bind(("row", RecordId::new("upload_ticket", ticket)))
        .await
        .expect("controlled DB expiry")
        .check()
        .expect("expiry mutation");
}

#[tokio::test]
async fn tickets_are_shared_single_use_bound_quota_limited_and_restart_durable() {
    let Some(mut server) = support::Server::start() else {
        return;
    };
    let repo = server.repo("uploads", "tickets").await;
    let registered = context("registration-one");
    let owner = binding("native-one", &registered);
    let input = request();
    let ticket = upload_ticket::issue(&repo, &owner, &input)
        .await
        .expect("issued destination");
    // A second gateway handle sees the same authoritative ticket immediately.
    let replica = server.repo("uploads", "tickets").await;
    let other = binding("native-other", &registered);
    assert!(refused(
        &upload_ticket::claim(&replica, &other, id(&ticket.upload_path)).await,
        "forbidden"
    ));
    let other_scope = UploadTicketBinding::new(
        "other-scope",
        "native-one",
        "access-epoch",
        CredentialPurpose::Native,
        &registered,
    )
    .expect("other scope");
    assert!(refused(
        &upload_ticket::claim(&replica, &other_scope, id(&ticket.upload_path)).await,
        "forbidden"
    ));
    let mut other_epoch_context = registered.clone();
    other_epoch_context.access_epoch = "other-epoch".into();
    let other_epoch = UploadTicketBinding::new(
        "scope",
        "native-one",
        "other-epoch",
        CredentialPurpose::Native,
        &other_epoch_context,
    )
    .expect("other epoch");
    assert!(refused(
        &upload_ticket::claim(&replica, &other_epoch, id(&ticket.upload_path)).await,
        "forbidden"
    ));
    let another_installation = binding("native-one", &context("registration-two"));
    assert!(refused(
        &upload_ticket::claim(&replica, &another_installation, id(&ticket.upload_path)).await,
        "forbidden"
    ));
    let mut changed = registered.clone();
    changed.registration_revision += 1;
    assert!(refused(
        &upload_ticket::claim(
            &replica,
            &binding("native-one", &changed),
            id(&ticket.upload_path)
        )
        .await,
        "forbidden"
    ));
    let mut changed = registered.clone();
    changed.configuration_epoch = "other-configuration".into();
    assert!(refused(
        &upload_ticket::claim(
            &replica,
            &binding("native-one", &changed),
            id(&ticket.upload_path)
        )
        .await,
        "forbidden"
    ));
    let operator = UploadTicketBinding::new(
        "scope",
        "native-one",
        "access-epoch",
        CredentialPurpose::Operator,
        &registered,
    )
    .expect("operator purpose");
    assert!(refused(
        &upload_ticket::claim(&replica, &operator, id(&ticket.upload_path)).await,
        "forbidden"
    ));
    let (first, second) = tokio::join!(
        upload_ticket::claim(&repo, &owner, id(&ticket.upload_path)),
        upload_ticket::claim(&replica, &owner, id(&ticket.upload_path))
    );
    assert_eq!(
        usize::from(first.is_ok()) + usize::from(second.is_ok()),
        1,
        "one shared-backend transaction wins a concurrent claim"
    );
    assert!(
        first.as_ref().is_ok_and(|value| value == &input)
            || second.as_ref().is_ok_and(|value| value == &input),
        "kind/hash/exact size remain pinned after claim"
    );
    assert!(refused(
        &upload_ticket::claim(&repo, &owner, id(&ticket.upload_path)).await,
        "unavailable"
    ));
    let pending = upload_ticket::issue(&repo, &owner, &input)
        .await
        .expect("pending ticket survives restart");
    drop(replica);
    drop(repo);
    server.restart();
    let repo = server.repo("uploads", "tickets").await;
    assert_eq!(
        upload_ticket::claim(&repo, &owner, id(&pending.upload_path))
            .await
            .expect("claim after process/datastore reopen"),
        input
    );
    assert!(
        refused(
            &upload_ticket::claim(&repo, &owner, id(&ticket.upload_path)).await,
            "unavailable"
        ),
        "claimed state survives reopen"
    );
    let expired = upload_ticket::issue(&repo, &owner, &input)
        .await
        .expect("expiring ticket");
    expire_one(&repo, id(&expired.upload_path)).await;
    assert!(refused(
        &upload_ticket::claim(&repo, &owner, id(&expired.upload_path)).await,
        "unavailable"
    ));
    // Claimed rows still consume the five-minute quota. Reset earlier fixture
    // lifetimes, then fill 63 and race the final principal capacity.
    repo.db()
        .query("UPDATE upload_ticket SET expires_at=time::now()-1s RETURN NONE")
        .await
        .expect("controlled prior expiry")
        .check()
        .expect("expiry");
    for _ in 0..upload_ticket::PER_ISSUER_LIMIT - 1 {
        upload_ticket::issue(&repo, &owner, &input)
            .await
            .expect("bounded pending quota");
    }
    let (first, second) = tokio::join!(
        upload_ticket::issue(&repo, &owner, &input),
        upload_ticket::issue(&repo, &owner, &input)
    );
    assert_eq!(
        usize::from(first.is_ok()) + usize::from(second.is_ok()),
        1,
        "issuer guard closes the quota race at 64"
    );
    assert!(refused(&first, "capacity") || refused(&second, "capacity"));
    assert!(refused(
        &upload_ticket::issue(&repo, &owner, &input).await,
        "capacity"
    ));
    assert!(
        upload_ticket::issue(&repo, &other, &input).await.is_ok(),
        "another principal does not share a global quota hotspot"
    );
    // Claim/fail loops cannot bypass quota; claimed rows remain counted.
    let mut pending=repo.db().query("SELECT VALUE record::id(id) FROM upload_ticket WHERE credential_id='native-one' AND state='pending' AND expires_at>time::now() LIMIT 1").await.expect("one pending").check().expect("pending query");
    let pending: Vec<String> = pending.take(0).expect("ticket ID");
    let pending_id = pending.first().expect("pending record");
    upload_ticket::claim(&repo, &owner, pending_id)
        .await
        .expect("actual single-use claim");
    assert!(
        refused(
            &upload_ticket::issue(&repo, &owner, &input).await,
            "capacity"
        ),
        "claiming does not bypass the per-principal issuance bound"
    );
    repo.db()
        .query("UPDATE upload_ticket SET expires_at=time::now()-1s RETURN NONE")
        .await
        .expect("controlled all-ticket expiry")
        .check()
        .expect("expiry");
    let removed = upload_ticket::expire(&repo)
        .await
        .expect("bounded metadata cleanup");
    assert_eq!(
        removed, 32,
        "one housekeeping invocation removes a bounded batch including claimed records"
    );
    assert!(
        upload_ticket::issue(&repo, &owner, &input).await.is_ok(),
        "expired entries do not block quota even before all cleanup batches finish"
    );
    let mut remaining = repo
        .db()
        .query("SELECT VALUE count() FROM upload_ticket WHERE credential_id='native-one' GROUP ALL")
        .await
        .expect("issuer retention")
        .check()
        .expect("issuer count");
    let remaining: Vec<i64> = remaining.take(0).expect("count");
    assert_eq!(
        remaining,
        vec![1],
        "active issuer prunes expired backlog within the same bounded issuance transaction"
    );
    assert!(refused(
        &upload_ticket::claim(&repo, &owner, "../../path").await,
        "unavailable"
    ));
    // The module rejects caller shapes that are not current Native/Operator
    // registration metadata; it never accepts a bearer or filesystem path.
    assert!(UploadTicketBinding::new(
        "scope",
        "web",
        "access-epoch",
        CredentialPurpose::WebRelay,
        &registered
    )
    .is_err());
    let mut invalid = registered;
    invalid.access_epoch = "other".into();
    assert!(UploadTicketBinding::new(
        "scope",
        "native-one",
        "access-epoch",
        CredentialPurpose::Native,
        &invalid
    )
    .is_err());
}
