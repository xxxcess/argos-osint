//! Origin-pinned public transport for native pairing, without bearer authority.

use std::fmt;
use std::time::Duration;

use reqwest::header;
use serde::de::DeserializeOwned;
use serde::Serialize;
use url::Url;

use super::ClientError;
use crate::pairing::PairingChallenge;
use crate::pairing::PairingPollRequest;
use crate::pairing::PairingStartRequest;
use crate::pairing::PairingStatus;
use crate::pairing::PAIRING_POLL_PATH;
use crate::pairing::PAIRING_START_PATH;

const MAX_PAIRING_REQUEST_BYTES: usize = 16 * 1024;
const MAX_PAIRING_RESPONSE_BYTES: usize = 16 * 1024;

/// Public pairing access pinned to one validated gateway origin.
///
/// Only start and poll are exposed. This client cannot read protected APIs or
/// approve a challenge. Remote origins require HTTPS; loopback may use HTTP.
/// Redirects and environment proxies are disabled, including for poll secrets.
#[derive(Clone)]
pub struct PairingClient {
    origin: Url,
    http: reqwest::Client,
}

impl fmt::Debug for PairingClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PairingClient")
            .finish_non_exhaustive()
    }
}

impl PairingClient {
    /// Validate the origin and create bounded transport without network access.
    pub fn new(origin: &str) -> Result<Self, ClientError> {
        let (origin, _) = super::validated_origin(origin)?;
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(15))
            .build()
            .map_err(ClientError::transport)?;
        Ok(Self { origin, http })
    }

    /// Start a challenge using a caller-generated private polling secret.
    pub async fn start(
        &self,
        request: &PairingStartRequest,
    ) -> Result<PairingChallenge, ClientError> {
        request
            .validate()
            .map_err(|_| ClientError::InvalidRequest)?;
        let challenge: PairingChallenge = self.post(PAIRING_START_PATH, request).await?;
        challenge
            .validate()
            .map_err(|_| ClientError::InvalidResponse)?;
        if challenge.pairing_id != request.request_id {
            return Err(ClientError::InvalidResponse);
        }
        Ok(challenge)
    }

    /// Poll the exact challenge without exposing its secret in a URL or header.
    pub async fn poll(&self, request: &PairingPollRequest) -> Result<PairingStatus, ClientError> {
        request
            .validate()
            .map_err(|_| ClientError::InvalidRequest)?;
        let status: PairingStatus = self.post(PAIRING_POLL_PATH, request).await?;
        status
            .validate()
            .map_err(|_| ClientError::InvalidResponse)?;
        Ok(status)
    }

    async fn post<T: DeserializeOwned>(
        &self,
        path: &str,
        request: &(impl Serialize + ?Sized),
    ) -> Result<T, ClientError> {
        let body = serde_json::to_vec(request).map_err(|_| ClientError::InvalidRequest)?;
        if body.len() > MAX_PAIRING_REQUEST_BYTES {
            return Err(ClientError::RequestTooLarge);
        }
        // All callers are private and use the two literal contract paths. Do not
        // expose generic unauthenticated route access through this transport.
        let url = self
            .origin
            .join(path)
            .map_err(|_| ClientError::InvalidRoute)?;
        let mut response = self
            .http
            .post(url)
            .header(header::ACCEPT, "application/json")
            .header(header::CONTENT_TYPE, "application/json")
            .body(body)
            .send()
            .await
            .map_err(ClientError::transport)?;
        if !response.status().is_success() {
            return Err(ClientError::Status(response.status().as_u16()));
        }
        if response
            .content_length()
            .is_some_and(|length| length > MAX_PAIRING_RESPONSE_BYTES as u64)
        {
            return Err(ClientError::ResponseTooLarge);
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(ClientError::transport)? {
            if chunk.len() > MAX_PAIRING_RESPONSE_BYTES - bytes.len() {
                return Err(ClientError::ResponseTooLarge);
            }
            bytes.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&bytes).map_err(|_| ClientError::InvalidResponse)
    }
}

#[cfg(test)]
mod tests {
    use tokio::io::AsyncReadExt;
    use tokio::io::AsyncWriteExt;
    use tokio::net::TcpListener;
    use tokio::task::JoinHandle;

    use super::*;

    #[test]
    fn public_pairing_retains_the_shared_origin_policy_and_redacted_debug() {
        for origin in [
            "http://example.invalid",
            "https://user:secret@example.invalid",
            "https://example.invalid/api",
            "https://example.invalid/?secret=poll-secret",
            "https://example.invalid/#poll-secret",
            "file:///tmp/poll-secret",
        ] {
            let error = PairingClient::new(origin).unwrap_err();
            assert_eq!(error, ClientError::InvalidOrigin);
            assert!(!format!("{error:?}: {error}").contains("poll-secret"));
        }
        let remote = PairingClient::new("https://private-gateway.invalid").unwrap();
        assert_eq!(format!("{remote:?}"), "PairingClient { .. }");
        assert!(PairingClient::new("http://127.0.0.1:8080").is_ok());
        assert!(PairingClient::new("http://[::1]:8080").is_ok());
    }

    #[tokio::test]
    async fn rejects_oversized_requests_before_network_access() {
        let client = PairingClient::new("http://127.0.0.1:1").unwrap();
        let result: Result<serde_json::Value, _> = client
            .post(PAIRING_POLL_PATH, &"x".repeat(MAX_PAIRING_REQUEST_BYTES))
            .await;
        assert_eq!(result, Err(ClientError::RequestTooLarge));
        let invalid = PairingPollRequest {
            pairing_id: "a".repeat(32),
            polling_secret: "weak".into(),
        };
        assert!(matches!(
            client.poll(&invalid).await,
            Err(ClientError::InvalidRequest)
        ));
    }

    #[tokio::test]
    async fn start_and_poll_use_the_public_contract_routes_and_validate_responses() {
        let start: PairingStartRequest = serde_json::from_value(serde_json::json!({
            "request_id": "a".repeat(32),
            "installation_marker": "synthetic-installation",
            "polling_secret": "a".repeat(64)
        }))
        .unwrap();
        let challenge = br#"{"pairing_id":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","approval_code":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","expires_at_ms":2000000000000,"poll_after_ms":1000}"#;
        let (origin, received) = serve_once("200 OK", "", challenge).await;
        let client = PairingClient::new(&origin).unwrap();
        let result = client.start(&start).await.unwrap();
        assert_eq!(result.pairing_id, start.request_id);
        assert!(received
            .await
            .unwrap()
            .starts_with(&format!("POST {PAIRING_START_PATH} HTTP/1.1\r\n")));

        let poll: PairingPollRequest = serde_json::from_value(serde_json::json!({
            "pairing_id": "a".repeat(32),
            "polling_secret": "a".repeat(64)
        }))
        .unwrap();
        let (origin, received) = serve_once(
            "200 OK",
            "",
            br#"{"status":"pending","poll_after_ms":1000}"#,
        )
        .await;
        let client = PairingClient::new(&origin).unwrap();
        assert!(matches!(
            client.poll(&poll).await,
            Ok(PairingStatus::Pending {
                poll_after_ms: 1000
            })
        ));
        assert!(received
            .await
            .unwrap()
            .starts_with(&format!("POST {PAIRING_POLL_PATH} HTTP/1.1\r\n")));

        let (origin, received) =
            serve_once("200 OK", "", br#"{"status":"pending","poll_after_ms":0}"#).await;
        let client = PairingClient::new(&origin).unwrap();
        assert!(matches!(
            client.poll(&poll).await,
            Err(ClientError::InvalidResponse)
        ));
        received.await.unwrap();

        let swapped = br#"{"pairing_id":"cccccccccccccccccccccccccccccccc","approval_code":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","expires_at_ms":2000000000000,"poll_after_ms":1000}"#;
        let (origin, received) = serve_once("200 OK", "", swapped).await;
        let client = PairingClient::new(&origin).unwrap();
        assert!(matches!(
            client.start(&start).await,
            Err(ClientError::InvalidResponse)
        ));
        received.await.unwrap();
    }

    #[tokio::test]
    async fn stalled_responses_return_a_safe_timeout() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (_socket, _) = listener.accept().await.unwrap();
            std::future::pending::<()>().await;
        });
        let (origin, _) = super::super::validated_origin(&origin).unwrap();
        let client = PairingClient {
            origin,
            http: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .no_proxy()
                .timeout(Duration::from_millis(50))
                .build()
                .unwrap(),
        };
        let result: Result<serde_json::Value, _> = client.post(PAIRING_POLL_PATH, &()).await;
        server.abort();
        assert!(server.await.unwrap_err().is_cancelled());
        assert_eq!(result, Err(ClientError::Timeout));
        assert_eq!(ClientError::Timeout.to_string(), "API request timed out");
    }

    #[tokio::test]
    async fn sends_only_json_without_bearer_context_or_secret_urls() {
        let (origin, received) = serve_once("200 OK", "", b"null").await;
        let client = PairingClient::new(&origin).unwrap();
        let result: serde_json::Value = client
            .post(
                PAIRING_POLL_PATH,
                &serde_json::json!({"poll_secret":"synthetic-poll-secret"}),
            )
            .await
            .unwrap();
        assert!(result.is_null());
        let request = received.await.unwrap();
        let (headers, body) = request.split_once("\r\n\r\n").unwrap();
        assert!(headers.starts_with(&format!("POST {PAIRING_POLL_PATH} HTTP/1.1\r\n")));
        assert!(!headers.contains("synthetic-poll-secret"));
        let headers = headers.to_ascii_lowercase();
        assert!(!headers.contains("authorization:"));
        assert!(!headers.contains("x-fleetworks-"));
        assert!(headers.contains("content-type: application/json"));
        assert!(body.contains("synthetic-poll-secret"));
    }

    #[tokio::test]
    async fn rejects_redirects_and_never_reflects_untrusted_diagnostics() {
        for (status, headers, body, expected) in [
            (
                "307 Temporary Redirect",
                "Location: https://elsewhere.invalid/poll-secret\r\n",
                "synthetic-secret",
                ClientError::Status(307),
            ),
            (
                "403 Forbidden",
                "",
                "synthetic-secret",
                ClientError::Status(403),
            ),
            (
                "200 OK",
                "",
                "synthetic-secret",
                ClientError::InvalidResponse,
            ),
        ] {
            let (origin, received) = serve_once(status, headers, body.as_bytes()).await;
            let client = PairingClient::new(&origin).unwrap();
            let result: Result<serde_json::Value, _> = client.post(PAIRING_START_PATH, &()).await;
            let error = result.unwrap_err();
            assert_eq!(error, expected);
            let diagnostic = format!("{error:?}: {error}");
            for private in [
                origin.as_str(),
                "synthetic-secret",
                "poll-secret",
                "elsewhere",
            ] {
                assert!(!diagnostic.contains(private));
            }
            received.await.unwrap();
        }
    }

    #[tokio::test]
    async fn bounds_both_announced_and_streamed_responses() {
        let announced = format!("Content-Length: {}\r\n", MAX_PAIRING_RESPONSE_BYTES + 1);
        for (headers, body) in [
            (announced, Vec::new()),
            (
                "Transfer-Encoding: chunked\r\n".into(),
                format!(
                    "{:x}\r\n{}\r\n0\r\n\r\n",
                    MAX_PAIRING_RESPONSE_BYTES + 1,
                    "x".repeat(MAX_PAIRING_RESPONSE_BYTES + 1)
                )
                .into_bytes(),
            ),
        ] {
            let (origin, received) = serve_once("200 OK", &headers, &body).await;
            let client = PairingClient::new(&origin).unwrap();
            let result: Result<serde_json::Value, _> = client.post(PAIRING_POLL_PATH, &()).await;
            assert_eq!(result, Err(ClientError::ResponseTooLarge));
            received.await.unwrap();
        }
    }

    async fn serve_once(status: &str, headers: &str, body: &[u8]) -> (String, JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let mut response = format!("HTTP/1.1 {status}\r\nConnection: close\r\n{headers}");
        if !headers.contains("Content-Length:") && !headers.contains("Transfer-Encoding:") {
            response.push_str(&format!("Content-Length: {}\r\n", body.len()));
        }
        response.push_str("\r\n");
        let mut response = response.into_bytes();
        response.extend_from_slice(body);
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut received = Vec::new();
            let header_end = loop {
                let mut byte = [0];
                socket.read_exact(&mut byte).await.unwrap();
                received.push(byte[0]);
                assert!(received.len() <= 8192);
                if received.ends_with(b"\r\n\r\n") {
                    break received.len();
                }
            };
            let headers = String::from_utf8(received.clone()).unwrap();
            let length: usize = headers
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .map(str::trim)
                        .map(str::to_owned)
                })
                .unwrap()
                .parse()
                .unwrap();
            assert!(length <= MAX_PAIRING_REQUEST_BYTES);
            received.resize(header_end + length, 0);
            socket
                .read_exact(&mut received[header_end..])
                .await
                .unwrap();
            socket.write_all(&response).await.unwrap();
            String::from_utf8(received).unwrap()
        });
        (origin, task)
    }
}
