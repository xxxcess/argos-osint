use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

use super::{Client, ClientError, MAX_REQUEST_BYTES, MAX_RESPONSE_BYTES};
use crate::{JOBS_PATH, READY_PATH};

#[tokio::test]
async fn registered_client_clone_keeps_global_reads_free_of_workspace_context() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let requests = tokio::spawn(async move {
        let mut received = Vec::new();
        for _ in 0..2 {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            while !bytes.ends_with(b"\r\n\r\n") {
                let mut byte = [0_u8; 1];
                assert_eq!(socket.read(&mut byte).await.unwrap(), 1);
                bytes.push(byte[0]);
                assert!(bytes.len() <= 16 * 1024);
            }
            received.push(String::from_utf8(bytes).unwrap().to_ascii_lowercase());
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\nnull")
                .await
                .unwrap();
        }
        received
    });
    let receipt = crate::connect::RegistrationReceipt::new(
        "deployment".into(),
        "node".into(),
        "registration-a".into(),
        None,
        3,
    );
    let context = crate::context::RegisteredContext::new(
        receipt,
        4,
        5,
        "fresh-configuration".into(),
        "access-epoch".into(),
    )
    .unwrap();
    let base = Client::new(&origin, Some("fixture-token")).unwrap();
    let scoped = base.with_registered_context(&context).unwrap();
    scoped.get::<serde_json::Value>(JOBS_PATH).await.unwrap();
    base.get::<serde_json::Value>(READY_PATH).await.unwrap();
    let received = requests.await.unwrap();
    assert!(received[0].contains("\r\nx-fleetworks-registration: registration-a\r\n"));
    assert!(received[0].contains("\r\nx-fleetworks-registration-revision: 3\r\n"));
    assert!(!received[1].contains("x-fleetworks-registration"));
    assert!(received
        .iter()
        .all(|request| request.contains("authorization: bearer fixture-token")));
}

#[test]
fn requires_a_safe_origin_and_token() {
    for origin in [
        "http://example.test",
        "http://user:private@localhost",
        "http://localhost/api/v1/",
        "http://localhost/?token=private",
        "http://localhost/#private",
        "file:///tmp/api",
    ] {
        assert_eq!(
            Client::new(origin, Some("fixture-token")).err(),
            Some(ClientError::InvalidOrigin),
            "{origin} must be rejected"
        );
    }
    assert_eq!(
        Client::new("https://example.test", None).err(),
        Some(ClientError::MissingToken)
    );
    assert_eq!(
        Client::new("http://localhost", Some("bad\r\nheader")).err(),
        Some(ClientError::InvalidToken)
    );
    assert!(Client::new("http://127.0.0.1:8080", None).is_ok());
    assert!(Client::new("https://example.test:8443", Some("fixture-token")).is_ok());
}

#[tokio::test]
async fn validates_routes_and_request_size_before_connecting() {
    let client = Client::new("http://127.0.0.1:1", None).unwrap();
    for route in [
        "//elsewhere.test/api/v1/jobs",
        "https://elsewhere.test/api/v1/jobs",
        "/api/v1/../jobs",
        "/api/v1/%2e%2e/jobs",
        "/api/v1/jobs?key=private",
        "/api/v1/jobs#private",
        "/api/v2/jobs",
    ] {
        assert_eq!(
            client.get::<serde_json::Value>(route).await,
            Err(ClientError::InvalidRoute),
            "{route} must be rejected"
        );
    }
    let oversized = "x".repeat(MAX_REQUEST_BYTES);
    assert_eq!(
        client
            .post::<serde_json::Value>(JOBS_PATH, &oversized)
            .await,
        Err(ClientError::RequestTooLarge)
    );
}

#[tokio::test]
async fn authorizes_the_configured_origin_and_encodes_query_values() {
    let body = br#"{"service":"argos-intel","api_version":1,"ready":true,"storage":true,"instance_id":"fixture"}"#;
    let (origin, request) = serve_once("200 OK", "", body).await;
    let client = Client::new(&origin, Some("fixture-token")).unwrap();
    let ready = client.ready().await.unwrap();
    assert!(ready.ready && ready.storage);
    assert_eq!(ready.instance_id, "fixture");
    let request = request.await.unwrap();
    assert!(request.starts_with("GET /api/v1/health/ready HTTP/1.1\r\n"));
    assert!(request
        .to_ascii_lowercase()
        .contains("\r\nauthorization: bearer fixture-token\r\n"));

    let (origin, request) = serve_once("200 OK", "", b"null").await;
    let client = Client::new(&origin, None).unwrap();
    client
        .get_with_query::<serde_json::Value>(JOBS_PATH, &[("id", "first /&=second")])
        .await
        .unwrap();
    let request = request.await.unwrap();
    assert!(request.starts_with("GET /api/v1/jobs?id=first+%2F%26%3Dsecond HTTP/1.1\r\n"));
    assert!(!request.to_ascii_lowercase().contains("authorization:"));
}

#[tokio::test]
async fn refuses_redirects_and_oversized_responses_without_reflecting_prose() {
    let (origin, request) = serve_once(
        "307 Temporary Redirect",
        "Location: /api/v1/health/ready\r\n",
        b"untrusted secret",
    )
    .await;
    let client = Client::new(&origin, Some("fixture-token")).unwrap();
    let error = client.ready().await.unwrap_err();
    assert_eq!(error, ClientError::Status(307));
    assert_eq!(error.to_string(), "API returned HTTP 307");
    request.await.unwrap();

    let oversized = format!("Content-Length: {}\r\n", MAX_RESPONSE_BYTES + 1);
    let (origin, request) = serve_once("200 OK", &oversized, b"").await;
    let client = Client::new(&origin, None).unwrap();
    assert_eq!(
        client.get::<serde_json::Value>(READY_PATH).await,
        Err(ClientError::ResponseTooLarge)
    );
    request.await.unwrap();
}

async fn serve_once(status: &str, headers: &str, body: &[u8]) -> (String, JoinHandle<String>) {
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let address = listener.local_addr().unwrap();
    let mut response = format!("HTTP/1.1 {status}\r\nConnection: close\r\n{headers}");
    if !headers.contains("Content-Length:") {
        response.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    response.push_str("\r\n");
    let mut response = response.into_bytes();
    response.extend_from_slice(body);
    let handle = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut received = Vec::new();
        let mut buffer = [0u8; 1024];
        while !received.windows(4).any(|part| part == b"\r\n\r\n") {
            let count = socket.read(&mut buffer).await.unwrap();
            assert!(
                count > 0 && received.len() + count <= 8192,
                "expected a bounded HTTP header"
            );
            received.extend_from_slice(&buffer[..count]);
        }
        socket.write_all(&response).await.unwrap();
        String::from_utf8(received).unwrap()
    });
    (format!("http://{address}"), handle)
}

async fn selected_upload_file(bytes: &[u8]) -> (std::path::PathBuf, tokio::fs::File) {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!(
        "fleetworks-upload-test-{}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
    ));
    let mut file = tokio::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&path)
        .await
        .unwrap();
    file.write_all(bytes).await.unwrap();
    file.flush().await.unwrap();
    (path, file)
}

async fn upload_request(socket: &mut tokio::net::TcpStream) -> (String, Vec<u8>) {
    let mut header = Vec::new();
    while !header.ends_with(b"\r\n\r\n") {
        header.push(socket.read_u8().await.unwrap());
        assert!(header.len() <= 8192);
    }
    let header = String::from_utf8(header).unwrap().to_ascii_lowercase();
    let size = header
        .lines()
        .find_map(|line| line.strip_prefix("content-length:"))
        .unwrap()
        .trim()
        .parse::<usize>()
        .unwrap();
    assert!(size < 256 * 1024, "synthetic upload fixture stays bounded");
    let mut body = vec![0; size];
    socket.read_exact(&mut body).await.unwrap();
    (header, body)
}

#[tokio::test]
async fn dataset_upload_requests_then_streams_only_the_issued_same_origin_path() {
    use sha2::Digest;

    let mut payload = crate::dataset_upload::ACTOR_CSV_COLUMNS.join(",") + "\n";
    for index in 0..2000 {
        payload.push_str(&format!("{index},Actor {index},person,[],actor {index}\n"));
    }
    let payload = payload.into_bytes();
    assert!(payload.len() > MAX_REQUEST_BYTES);
    let sha256 = format!("{:x}", sha2::Sha256::digest(&payload));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let expected = payload.clone();
    let hash = sha256.clone();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let (start, body) = upload_request(&mut socket).await;
        assert!(start.starts_with("post /api/v1/workstation/datasets/uploads http/1.1\r\n"));
        let metadata: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(metadata["kind"], "actor_catalog");
        assert_eq!(metadata["sha256"], hash);
        assert_eq!(metadata["size_bytes"], expected.len());
        let ticket = br#"{"upload_path":"/api/v1/workstation/datasets/uploads/issued-id","expires_in_seconds":300}"#;
        socket
            .write_all(
                format!(
                    "HTTP/1.1 201 Created\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    ticket.len()
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        socket.write_all(ticket).await.unwrap();
        drop(socket);
        let (mut socket, _) = listener.accept().await.unwrap();
        let (put, bytes) = upload_request(&mut socket).await;
        assert!(put.starts_with("put /api/v1/workstation/datasets/uploads/issued-id http/1.1\r\n"));
        assert!(put.contains("\r\ncontent-type: text/csv\r\n"));
        assert_eq!(bytes, expected);
        for headers in [&start, &put] {
            assert!(headers.contains("\r\nauthorization: bearer fixture-token\r\n"));
            assert!(headers.contains("\r\nx-fleetworks-registration: logs-registration\r\n"));
            assert!(headers.contains("\r\nx-fleetworks-registration-revision: 3\r\n"));
            assert!(!headers.lines().next().unwrap().contains('?'));
        }
        let response = serde_json::json!({"dataset":{"id":"dataset-issued","kind":"actor_catalog","sha256":hash,"size_bytes":expected.len()},"expires_in_seconds":86400}).to_string();
        socket
            .write_all(
                format!(
                    "HTTP/1.1 201 Created\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    response.len()
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        socket.write_all(response.as_bytes()).await.unwrap();
    });
    let (path, file) = selected_upload_file(&payload).await;
    let uploaded = log_client(&origin)
        .upload_dataset(
            crate::readiness::DatasetKind::ActorCatalog,
            &sha256,
            file,
            payload.len() as u64,
        )
        .await
        .unwrap();
    assert_eq!(uploaded.dataset.id, "dataset-issued");
    tokio::time::timeout(std::time::Duration::from_secs(5), server)
        .await
        .unwrap()
        .unwrap();
    tokio::fs::remove_file(path).await.unwrap();
}

#[tokio::test]
async fn dataset_upload_rejects_untrusted_ticket_before_streaming() {
    for path in [
        "https://other.invalid/api/v1/workstation/datasets/uploads/id",
        "//other.invalid/api/v1/workstation/datasets/uploads/id",
        "/api/v1/workstation/datasets/uploads/id?secret=private",
        "/api/v1/workstation/datasets/uploads/../id",
    ] {
        let body = serde_json::json!({"upload_path":path,"expires_in_seconds":300}).to_string();
        let (origin, request) = serve_once("201 Created", "", body.as_bytes()).await;
        let (path, file) = selected_upload_file(b"synthetic").await;
        assert_eq!(
            log_client(&origin)
                .upload_dataset(
                    crate::readiness::DatasetKind::ActorCatalog,
                    &"a".repeat(64),
                    file,
                    9
                )
                .await
                .unwrap_err(),
            ClientError::InvalidResponse
        );
        assert!(request
            .await
            .unwrap()
            .starts_with("POST /api/v1/workstation/datasets/uploads HTTP/1.1\r\n"));
        tokio::fs::remove_file(path).await.unwrap();
    }
}

#[tokio::test]
async fn dataset_upload_requires_registration_and_exact_local_size_before_network() {
    let (path, file) = selected_upload_file(b"synthetic").await;
    let client = Client::new("http://127.0.0.1:1", None).unwrap();
    assert_eq!(
        client
            .upload_dataset(
                crate::readiness::DatasetKind::ActorCatalog,
                &"a".repeat(64),
                file,
                9
            )
            .await
            .unwrap_err(),
        ClientError::InvalidRequest
    );
    tokio::fs::remove_file(path).await.unwrap();
    let (path, file) = selected_upload_file(b"synthetic").await;
    assert_eq!(
        log_client("http://127.0.0.1:1")
            .upload_dataset(
                crate::readiness::DatasetKind::ActorCatalog,
                &"a".repeat(64),
                file,
                8
            )
            .await
            .unwrap_err(),
        ClientError::InvalidRequest
    );
    tokio::fs::remove_file(path).await.unwrap();
}

#[test]
fn canonical_profile_origin_reuses_transport_policy_without_requiring_credentials() {
    assert_eq!(
        Client::canonical_origin("https://EXAMPLE.invalid:443")
            .expect("offline origin normalization"),
        "https://example.invalid/"
    );
    assert_eq!(
        Client::canonical_origin("http://127.0.0.1:80").expect("loopback normalization"),
        "http://127.0.0.1/"
    );
    assert!(matches!(
        Client::new("https://example.invalid", None),
        Err(ClientError::MissingToken)
    ));
    for origin in [
        "http://192.0.2.1",
        "https://user:secret@example.invalid",
        "https://example.invalid/api",
        "https://example.invalid?secret=value",
        "https://example.invalid/#fragment",
        "file:///tmp/profile",
    ] {
        assert_eq!(
            Client::canonical_origin(origin),
            Err(ClientError::InvalidOrigin)
        );
        assert!(matches!(
            Client::new(origin, Some("synthetic-token")),
            Err(ClientError::InvalidOrigin)
        ));
    }
}

fn log_client(origin: &str) -> Client {
    let receipt = crate::connect::RegistrationReceipt::new(
        "deployment".into(),
        "node".into(),
        "logs-registration".into(),
        None,
        3,
    );
    let context = crate::context::RegisteredContext::new(
        receipt,
        4,
        5,
        "current-configuration".into(),
        "current-access".into(),
    )
    .unwrap();
    Client::new(origin, Some("fixture-token"))
        .unwrap()
        .with_registered_context(&context)
        .unwrap()
}

#[tokio::test]
async fn log_export_is_registered_bounded_and_reports_incomplete_snapshot() {
    let body = br#"{"id":"event","ts":"2026-11-12T12:00:00Z","severity":"info","app":"tools","event_type":"phase","message":"synthetic private summary","job_id":"job","task_id":"","attempt_id":"","run_id":""}
"#;
    let cursor = "ab".repeat(32);
    let headers = format!("Content-Type: application/x-ndjson\r\nContent-Disposition: attachment; filename=workstation-logs.ndjson\r\nx-fleetworks-log-apps-complete: false\r\nx-fleetworks-next-log-cursor: {cursor}\r\n");
    let (origin, request) = serve_once("200 OK", &headers, body).await;
    let query = crate::operational_logs::LogQuery {
        text: Some("first /&=second".into()),
        limit: Some(1),
        ..Default::default()
    };
    let export = log_client(&origin).export_logs(&query).await.unwrap();
    assert_eq!(export.bytes, body);
    assert_eq!(export.next_cursor.as_deref(), Some(cursor.as_str()));
    assert!(!export.apps_complete);
    assert!(!format!("{export:?}").contains("synthetic private summary"));
    let request = request.await.unwrap().to_ascii_lowercase();
    assert!(request
        .starts_with("get /api/v1/logs/export?text=first+%2f%26%3dsecond&limit=1 http/1.1\r\n"));
    assert!(request.contains("\r\nx-fleetworks-registration: logs-registration\r\n"));
    assert!(request.contains("\r\nauthorization: bearer fixture-token\r\n"));
}

#[tokio::test]
async fn log_export_rejects_raw_payloads_bad_metadata_and_oversize() {
    let headers = "Content-Type: application/x-ndjson\r\nContent-Disposition: attachment; filename=workstation-logs.ndjson\r\nx-fleetworks-log-apps-complete: true\r\n";
    let body = br#"{"id":"event","ts":"2026-11-12T12:00:00Z","severity":"info","app":"tools","event_type":"phase","message":"safe","job_id":"job","task_id":"","attempt_id":"","run_id":"","details":{"secret":"synthetic-never-exported"}}"#;
    for (response_headers, bytes, expected) in [
        (
            headers.to_owned(),
            body.as_slice(),
            ClientError::InvalidResponse,
        ),
        (
            headers.replace("x-fleetworks-log-apps-complete: true\r\n", ""),
            b"".as_slice(),
            ClientError::InvalidResponse,
        ),
        (
            format!("{headers}x-fleetworks-next-log-cursor: raw-sequence-42\r\n"),
            b"".as_slice(),
            ClientError::InvalidResponse,
        ),
        (
            format!("{headers}Content-Length: {}\r\n", MAX_RESPONSE_BYTES + 1),
            b"".as_slice(),
            ClientError::ResponseTooLarge,
        ),
    ] {
        let (origin, request) = serve_once("200 OK", &response_headers, bytes).await;
        assert_eq!(
            log_client(&origin)
                .export_logs(&Default::default())
                .await
                .unwrap_err(),
            expected
        );
        request.await.unwrap();
    }
}

#[tokio::test]
async fn log_export_refuses_missing_context_and_untrusted_error_prose() {
    let unregistered = Client::new("http://127.0.0.1:1", None).unwrap();
    assert_eq!(
        unregistered
            .export_logs(&Default::default())
            .await
            .unwrap_err(),
        ClientError::InvalidRequest
    );
    let (origin, request) = serve_once(
        "409 Conflict",
        "",
        b"untrusted synthetic private diagnostic",
    )
    .await;
    let error = log_client(&origin)
        .export_logs(&Default::default())
        .await
        .unwrap_err();
    assert_eq!(error, ClientError::Status(409));
    assert!(!error.to_string().contains("synthetic"));
    request.await.unwrap();
}
