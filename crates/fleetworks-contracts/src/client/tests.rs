use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

use super::{Client, ClientError, MAX_REQUEST_BYTES, MAX_RESPONSE_BYTES};
use crate::{JOBS_PATH, READY_PATH};

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
