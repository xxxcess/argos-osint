//! Issued destinations are streamed, bounded and registration-bound.
use argos_osint_core::storage::bootstrap;
use argos_osint_core::storage::connect_registry;
#[cfg(feature = "embedded")]
use argos_osint_core::storage::repo::Repository;
use axum::body::Body;
use axum::http::Request;
use axum::Router;
use fleetworks_contracts::connect::RegistrationEnsureRequest;
use fleetworks_contracts::connect::RegistrationReceipt;
use serde_json::Value;
use tower::ServiceExt;

use super::*;
use crate::NetworkPolicy;
use crate::PrivateServices;
use crate::ScopedCredential;

const HOST: &str = "upload.test:8080";
const TOKEN: &str = "synthetic-native-upload-0123456789abcdef";
const OTHER_TOKEN: &str = "synthetic-other-native-0123456789abcdef";
fn policy() -> NetworkPolicy {
    let credentials = [("native", TOKEN), ("other", OTHER_TOKEN)]
        .into_iter()
        .map(|(id, token)| {
            ScopedCredential::new(
                id.into(),
                "upload-scope".into(),
                "upload-epoch".into(),
                CredentialPurpose::Native,
                [
                    Permission::ReadBackend,
                    Permission::RegisterWorkstation,
                    Permission::MaintainBackend,
                ]
                .into_iter()
                .collect(),
                token,
            )
            .expect("purpose policy")
        })
        .collect();
    NetworkPolicy::with_credentials(&[HOST.into()], credentials).expect("explicit policy")
}
async fn call(
    app: &Router,
    method: &str,
    path: &str,
    registration: Option<&RegistrationReceipt>,
    token: &str,
    body: Body,
    content_type: &str,
) -> (StatusCode, Value) {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header("host", HOST)
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", content_type);
    if let Some(registration) = registration {
        request = request
            .header(
                "x-fleetworks-registration",
                registration.registration_id.as_str(),
            )
            .header(
                "x-fleetworks-registration-revision",
                registration.revision.to_string(),
            );
    }
    let response = app
        .clone()
        .oneshot(request.body(body).expect("request"))
        .await
        .expect("response");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .expect("bounded response");
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}
fn metadata(bytes: &[u8], size: u64) -> Body {
    Body::from(json!({"kind":"actor_catalog","sha256":format!("{:x}",Sha256::digest(bytes)),"size_bytes":size}).to_string())
}
fn stream(bytes: Vec<u8>) -> Body {
    let chunks: Vec<_> = bytes
        .chunks(1024)
        .map(|bytes| Ok::<_, std::io::Error>(Bytes::copy_from_slice(bytes)))
        .collect();
    Body::from_stream(futures_util::stream::iter(chunks))
}
async fn issue(
    app: &Router,
    registration: &RegistrationReceipt,
    bytes: &[u8],
    size: u64,
) -> String {
    let (status, ticket) = call(
        app,
        "POST",
        fleetworks_contracts::dataset_upload::DATASET_UPLOADS_PATH,
        Some(registration),
        TOKEN,
        metadata(bytes, size),
        "application/json",
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "registered native may request a destination"
    );
    assert_eq!(
        ticket["expires_in_seconds"],
        upload_ticket::TICKET_TTL.as_secs()
    );
    let path = ticket["upload_path"]
        .as_str()
        .expect("relative destination");
    assert!(path.starts_with("/api/v1/workstation/datasets/uploads/"));
    path.into()
}

#[tokio::test]
async fn issued_upload_urls_stream_exact_bytes_once_and_preserve_registration_fences() {
    let directory = tempfile::tempdir().expect("private staging parent");
    let root = directory.path().canonicalize().expect("canonical parent");
    #[cfg(feature = "embedded")]
    let repo = Repository::connect(
        &argos_osint_core::storage::open::Target::Embedded {
            path: root.join("store"),
        },
        "uploads",
        "issued",
    )
    .await
    .expect("isolated repository");
    #[cfg(not(feature = "embedded"))]
    let datastore =
        crate::storage_test_support::Server::start().expect("required pinned isolated server");
    #[cfg(not(feature = "embedded"))]
    let repo = datastore.repo("uploads", "issued").await;
    bootstrap::ensure_network_identity(repo.db())
        .await
        .expect("identity");
    let deployment = connect_registry::initialize_control(repo.db())
        .await
        .expect("control");
    let authority =
        RegistryAuthority::new("upload-scope", "native", "upload-epoch").expect("native authority");
    let mut registrations = Vec::new();
    for marker in ["synthetic-installation-one", "synthetic-installation-two"] {
        registrations.push(
            connect_registry::ensure(
                &repo,
                &authority,
                &RegistrationEnsureRequest::new(format!("ensure-{marker}"), None, marker.into()),
            )
            .await
            .expect("independent installation")
            .receipt,
        );
    }
    assert_ne!(
        registrations[0].workstation_id,
        registrations[1].workstation_id
    );
    let registration = &registrations[0];
    let store = DatasetStore::open(root.join("uploads")).expect("staging store");
    let state = ApiState {
        repo: repo.clone(),
        instance_id: "upload-fixture".into(),
        port: 8080,
        origins: Arc::new(Vec::new()),
    };
    let services = || {
        PrivateServices::new(None, None, None, None).with_workstation(
            crate::workstation::ReadinessOwners::new(deployment.deployment_id.clone(), Vec::new())
                .expect("readiness metadata"),
            store.clone(),
        )
    };
    let app = crate::network_router_with_private_services(state.clone(), policy(), services());
    let mut bytes = b"actor_id,actor_name,actor_type,aliases,name_normalized\n".to_vec();
    for n in 0..400 {
        bytes.extend_from_slice(
            format!("synthetic-{n},Harbor Union {n},organization,[],harbor union {n}\n").as_bytes(),
        );
    }
    assert!(
        bytes.len() > crate::MAX_REQUEST_BYTES,
        "PUT fixture crosses the ordinary JSON limit"
    );
    let path = issue(&app, registration, &bytes, bytes.len() as u64).await;
    assert_eq!(
        call(
            &app,
            "PUT",
            &path,
            Some(registration),
            OTHER_TOKEN,
            stream(bytes.clone()),
            "text/csv"
        )
        .await
        .0,
        StatusCode::FORBIDDEN,
        "another credential cannot claim the registration's URL"
    );
    assert_eq!(
        call(
            &app,
            "PUT",
            &path,
            Some(&registrations[1]),
            TOKEN,
            stream(bytes.clone()),
            "text/csv"
        )
        .await
        .0,
        StatusCode::FORBIDDEN,
        "another installation cannot claim the URL"
    );
    assert_eq!(
        call(
            &app,
            "PUT",
            &path,
            None,
            TOKEN,
            stream(bytes.clone()),
            "text/csv"
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED,
        "URL alone never grants registration authority"
    );
    let (status, result) = call(
        &app,
        "PUT",
        &path,
        Some(registration),
        TOKEN,
        stream(bytes.clone()),
        "text/csv",
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "issued PUT streams beyond the small JSON limit"
    );
    let reference: DatasetRef =
        serde_json::from_value(result["dataset"].clone()).expect("immutable reference");
    assert_eq!(reference.size_bytes, bytes.len() as u64);
    assert_eq!(reference.sha256, format!("{:x}", Sha256::digest(&bytes)));
    assert_eq!(
        setup_jobs::dataset(&repo, &authority, &reference.id)
            .await
            .expect("durable metadata"),
        reference
    );
    assert_eq!(
        tokio::fs::read(store.path(&reference.id, "csv").expect("owned path"))
            .await
            .expect("staged bytes"),
        bytes
    );
    assert_eq!(
        call(
            &app,
            "PUT",
            &path,
            Some(registration),
            TOKEN,
            stream(bytes.clone()),
            "text/csv"
        )
        .await
        .0,
        StatusCode::GONE,
        "used URL cannot mint another reference"
    );
    let (_, list) = call(
        &app,
        "GET",
        "/api/v1/workstation/datasets",
        Some(registration),
        TOKEN,
        Body::empty(),
        "application/json",
    )
    .await;
    assert_eq!(list["rows"].as_array().expect("rows").len(), 1);
    let mut jobs = repo
        .db()
        .query("RETURN count(SELECT VALUE id FROM job)")
        .await
        .expect("jobs query")
        .check()
        .expect("jobs");
    let count: Option<i64> = jobs.take(0).expect("job count");
    assert_eq!(
        count.unwrap_or(0),
        0,
        "staging never submits seed/train jobs or activation"
    );
    // Exact size is checked even without an HTTP Content-Length header.
    for declared in [bytes.len() as u64 - 1, bytes.len() as u64 + 1] {
        let path = issue(&app, registration, &bytes, declared).await;
        assert_eq!(
            call(
                &app,
                "PUT",
                &path,
                Some(registration),
                TOKEN,
                stream(bytes.clone()),
                "text/csv"
            )
            .await
            .0,
            StatusCode::UNPROCESSABLE_ENTITY
        );
        assert_eq!(
            call(
                &app,
                "PUT",
                &path,
                Some(registration),
                TOKEN,
                stream(bytes.clone()),
                "text/csv"
            )
            .await
            .0,
            StatusCode::GONE
        );
    }
    let wrong = b"not,the,actor,header\nsynthetic,values,remain,invalid\n";
    let path = issue(&app, registration, wrong, wrong.len() as u64).await;
    assert_eq!(
        call(
            &app,
            "PUT",
            &path,
            Some(registration),
            TOKEN,
            stream(wrong.to_vec()),
            "text/csv"
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY,
        "server repeats shared header validation"
    );
    let path = issue(&app, registration, &bytes, bytes.len() as u64).await;
    let interrupted = Body::from_stream(futures_util::stream::iter([
        Ok(Bytes::from_static(b"actor_id,")),
        Err(std::io::Error::other("synthetic stream failure")),
    ]));
    assert_eq!(
        call(
            &app,
            "PUT",
            &path,
            Some(registration),
            TOKEN,
            interrupted,
            "text/csv"
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let path = issue(&app, registration, &bytes, bytes.len() as u64).await;
    repo.db()
        .query("UPDATE $ticket SET expires_at=time::now()-1s RETURN NONE")
        .bind((
            "ticket",
            surrealdb::types::RecordId::new("upload_ticket", path.rsplit('/').next().expect("id")),
        ))
        .await
        .expect("controlled expired ticket")
        .check()
        .expect("expiry");
    assert_eq!(
        call(
            &app,
            "PUT",
            &path,
            Some(registration),
            TOKEN,
            stream(bytes.clone()),
            "text/csv"
        )
        .await
        .0,
        StatusCode::GONE
    );
    let path = issue(&app, registration, &bytes, bytes.len() as u64).await;
    let restarted_store =
        DatasetStore::open(root.join("uploads")).expect("same durable staging root");
    let restarted = crate::network_router_with_private_services(
        state,
        policy(),
        PrivateServices::new(None, None, None, None).with_workstation(
            crate::workstation::ReadinessOwners::new(deployment.deployment_id.clone(), Vec::new())
                .expect("readiness"),
            restarted_store,
        ),
    );
    assert_eq!(
        call(
            &restarted,
            "PUT",
            &path,
            Some(registration),
            TOKEN,
            stream(bytes.clone()),
            "text/csv"
        )
        .await
        .0,
        StatusCode::CREATED,
        "another gateway instance claims the persisted destination"
    );
    assert_eq!(
        setup_jobs::dataset(&repo, &authority, &reference.id)
            .await
            .expect("durable reference survives restart"),
        reference
    );
    repo.db()
        .query("UPDATE upload_ticket SET expires_at=time::now()-1s RETURN NONE")
        .await
        .expect("controlled prior ticket expiry")
        .check()
        .expect("expiry");
    for _ in 0..upload_ticket::PER_ISSUER_LIMIT {
        issue(&app, registration, &bytes, bytes.len() as u64).await;
    }
    assert_eq!(
        call(
            &app,
            "POST",
            fleetworks_contracts::dataset_upload::DATASET_UPLOADS_PATH,
            Some(registration),
            TOKEN,
            metadata(&bytes, bytes.len() as u64),
            "application/json"
        )
        .await
        .0,
        StatusCode::TOO_MANY_REQUESTS
    );
    repo.db()
        .query("UPDATE upload_ticket SET expires_at=time::now()-1s RETURN NONE")
        .await
        .expect("controlled quota expiry")
        .check()
        .expect("expiry");
    issue(&app, registration, &bytes, bytes.len() as u64).await;
    let mut live = repo
        .db()
        .query("SELECT VALUE count() FROM upload_ticket WHERE expires_at>time::now() GROUP ALL")
        .await
        .expect("quota observation")
        .check()
        .expect("live quota");
    let live: Vec<i64> = live.take(0).expect("count");
    assert_eq!(
        live,
        vec![1],
        "expired tickets release issuer quota before housekeeping"
    );
    let large_json=Body::from(json!({"kind":"actor_catalog","sha256":"a".repeat(64),"size_bytes":1,"padding":"x".repeat(crate::MAX_REQUEST_BYTES+1)}).to_string());
    assert_eq!(
        call(
            &app,
            "POST",
            fleetworks_contracts::dataset_upload::DATASET_UPLOADS_PATH,
            Some(registration),
            TOKEN,
            large_json,
            "application/json"
        )
        .await
        .0,
        StatusCode::PAYLOAD_TOO_LARGE,
        "POST metadata retains the small JSON limit"
    );
}

#[test]
fn actor_upload_validation_is_record_bounded_and_cancellable() {
    let directory = tempfile::tempdir().expect("private parser fixture");
    let path = directory.path().join("actors.csv");
    let mut bytes = b"actor_id,actor_name,actor_type,aliases,name_normalized\n".to_vec();
    bytes.extend_from_slice(
        format!(
            "actor,{},organization,[],synthetic\n",
            "x".repeat(70 * 1024)
        )
        .as_bytes(),
    );
    std::fs::write(&path, bytes).expect("oversized raw record fixture");
    assert!(
        validate_csv(&path, DatasetKind::ActorCatalog, &AtomicBool::new(false)).is_err(),
        "upload validation uses the same 64KiB raw-record budget as the actual seed worker"
    );
    std::fs::write(&path,b"actor_id,actor_name,actor_type,aliases,name_normalized\nsynthetic,Harbor Union,organization,[],harbor union\n").expect("valid actor fixture");
    assert!(
        validate_csv(&path, DatasetKind::ActorCatalog, &AtomicBool::new(true)).is_err(),
        "cancelled validation never publishes a DatasetRef"
    );
    assert!(validate_csv(&path, DatasetKind::ActorCatalog, &AtomicBool::new(false)).is_ok());
}

#[tokio::test]
async fn cancelled_http_owner_cannot_release_a_running_validation_slot() {
    let slots = Arc::new(Semaphore::new(1));
    let owner = Arc::new(slots.clone().try_acquire_owned().expect("stream slot"));
    let (entered, ready) = tokio::sync::oneshot::channel();
    let (release, wait) = std::sync::mpsc::channel();
    let worker = spawn_validation(Arc::clone(&owner), move || {
        entered.send(()).expect("test still waiting");
        wait.recv().expect("deterministic release barrier");
        Ok(())
    });
    ready.await.expect("actual blocking worker entered");
    // This is the owner dropped by a cancelled HTTP stage; the production
    // validation wrapper independently holds capacity until CPU work settles.
    drop(owner);
    let capacity_retained = slots.clone().try_acquire_owned().is_err();
    // Always release/join before asserting: a regression must fail the test,
    // rather than strand a blocking worker during runtime shutdown.
    release.send(()).expect("release actual worker");
    worker
        .await
        .expect("joined actual blocking worker")
        .expect("validation complete");
    assert!(
        capacity_retained,
        "disconnect cannot oversubscribe parser capacity"
    );
    assert!(
        slots.try_acquire_owned().is_ok(),
        "completed CPU work releases its slot"
    );
    let flag = Arc::new(AtomicBool::new(false));
    let cancellation = CancelValidation(Arc::clone(&flag));
    drop(cancellation);
    assert!(
        flag.load(Ordering::Acquire),
        "HTTP stage cancellation reaches the bounded actor parser"
    );
}
