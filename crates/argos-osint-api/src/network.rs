//! Ingress policy shared by the HTTP routes and the WebSocket upgrade.

use std::sync::Arc;

use anyhow::ensure;
use axum::extract::{Request, State};
use axum::http::header;
use axum::http::uri::Authority;
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

const MIN_TOKEN_LEN: usize = 32;
const MAX_TOKEN_LEN: usize = 256;

/// Explicit opt-in ingress policy for the Fleetworks API.
#[derive(Clone)]
pub struct NetworkPolicy {
    hosts: Vec<Authority>,
    token_digest: [u8; 32],
}

impl NetworkPolicy {
    /// Accepts exact Host authorities and a 32–256 byte bearer token.
    ///
    /// Include the published port in each authority when clients send it.
    pub fn new(hosts: &[String], token: &str) -> anyhow::Result<Self> {
        ensure!(!hosts.is_empty(), "at least one API host is required");
        ensure!(
            valid_token(token.as_bytes()),
            "API bearer token must be 32–256 bytes in bearer token syntax"
        );
        let hosts = hosts
            .iter()
            .map(|host| {
                let authority: Authority = host
                    .parse()
                    .map_err(|_| anyhow::anyhow!("invalid API host authority"))?;
                let port_suffix = &authority.as_str()[authority.host().len()..];
                let valid_port = port_suffix.is_empty()
                    || port_suffix
                        .strip_prefix(':')
                        .is_some_and(|port| port.parse::<u16>().is_ok_and(|number| number != 0));
                ensure!(
                    !authority.host().is_empty() && !host.contains('@') && valid_port,
                    "invalid API host authority"
                );
                Ok(authority)
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        Ok(Self {
            hosts,
            token_digest: Sha256::digest(token).into(),
        })
    }

    fn allows_host(&self, host: &str) -> bool {
        self.hosts
            .iter()
            .any(|allowed| allowed.as_str().eq_ignore_ascii_case(host))
    }

    fn authorized(&self, headers: &HeaderMap) -> bool {
        let values = headers.get_all(header::AUTHORIZATION);
        let mut values = values.iter();
        let Some(value) = values.next() else {
            return false;
        };
        let bytes = value.as_bytes();
        if values.next().is_some()
            || bytes.len() < 7
            || !bytes[..7].eq_ignore_ascii_case(b"Bearer ")
        {
            return false;
        }
        let token = &bytes[7..];
        if !valid_token(token) {
            return false;
        }
        let digest: [u8; 32] = Sha256::digest(token).into();
        bool::from(self.token_digest.ct_eq(&digest))
    }
}

pub(crate) fn valid_token(token: &[u8]) -> bool {
    if !(MIN_TOKEN_LEN..=MAX_TOKEN_LEN).contains(&token.len()) {
        return false;
    }
    let value = token.trim_ascii_end();
    let end = value.iter().rposition(|byte| *byte != b'=');
    end.is_some_and(|end| {
        value.len() == token.len()
            && value[..=end].iter().all(|byte| {
                byte.is_ascii_alphanumeric()
                    || matches!(byte, b'-' | b'.' | b'_' | b'~' | b'+' | b'/')
            })
    })
}

/// Validates and canonicalizes an exact HTTP(S) browser origin.
pub fn network_origin(raw: &str) -> anyhow::Result<String> {
    Ok(validate_origin(raw)?.origin().ascii_serialization())
}

pub(crate) fn validate_origin(raw: &str) -> anyhow::Result<url::Url> {
    let origin = url::Url::parse(raw).map_err(|_| anyhow::anyhow!("invalid API origin"))?;
    ensure!(
        matches!(origin.scheme(), "http" | "https")
            && origin.host_str().is_some()
            && origin.username().is_empty()
            && origin.password().is_none()
            && origin.path() == "/"
            && origin.query().is_none()
            && origin.fragment().is_none(),
        "invalid API origin"
    );
    Ok(origin)
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AccessMode {
    Loopback,
    Bearer,
}

#[derive(Clone)]
pub(crate) struct AccessPolicy {
    pub(crate) port: u16,
    pub(crate) origins: Arc<Vec<String>>,
    pub(crate) network: Option<NetworkPolicy>,
}

pub(crate) async fn protect(
    State(policy): State<AccessPolicy>,
    request: Request,
    next: Next,
) -> Response {
    let headers = request.headers();
    let host = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    let allowed_host = match &policy.network {
        Some(network) => {
            headers.get_all(header::HOST).iter().count() == 1 && network.allows_host(host)
        }
        None => {
            host == format!("127.0.0.1:{}", policy.port)
                || host == format!("localhost:{}", policy.port)
                || (policy.port == 80 && matches!(host, "127.0.0.1" | "localhost"))
        }
    };
    if !allowed_host {
        return refused(StatusCode::FORBIDDEN, "Host refused");
    }

    let origin = headers.get(header::ORIGIN).cloned();
    if let Some(value) = &origin {
        if (policy.network.is_some() && headers.get_all(header::ORIGIN).iter().count() != 1)
            || !policy
                .origins
                .iter()
                .any(|allowed| value.as_bytes() == allowed.as_bytes())
        {
            return refused(StatusCode::FORBIDDEN, "Origin refused");
        }
    }
    if headers
        .get("sec-fetch-site")
        .is_some_and(|value| value == "cross-site")
        && origin.is_none()
    {
        return refused(StatusCode::FORBIDDEN, "Origin required");
    }

    let mut response = if request.method() == Method::OPTIONS {
        StatusCode::NO_CONTENT.into_response()
    } else if policy
        .network
        .as_ref()
        .is_some_and(|network| !network.authorized(headers))
    {
        let mut response = refused(StatusCode::UNAUTHORIZED, "API authentication required");
        response
            .headers_mut()
            .insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
        response
    } else {
        next.run(request).await
    };
    let headers = response.headers_mut();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(header::VARY, HeaderValue::from_static("Origin"));
    if let Some(value) = origin {
        headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, value);
        headers.insert(
            header::ACCESS_CONTROL_ALLOW_METHODS,
            HeaderValue::from_static("GET, POST, OPTIONS"),
        );
        let allowed = if policy.network.is_some() {
            "authorization, content-type"
        } else {
            "content-type"
        };
        headers.insert(
            header::ACCESS_CONTROL_ALLOW_HEADERS,
            HeaderValue::from_static(allowed),
        );
    }
    response
}

fn refused(status: StatusCode, reason: &'static str) -> Response {
    (status, Json(json!({"error": reason}))).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use axum::middleware;
    use axum::routing::get;
    use axum::Router;
    use tower::ServiceExt;

    const TOKEN: &str = "0123456789abcdefghijklmnopqrstuvwxyz";

    #[test]
    fn rejects_invalid_config_without_reflecting_it() {
        for host in ["", "https://api:8080", "name@api:8080", "api:70000"] {
            let result = NetworkPolicy::new(&[host.into()], TOKEN);
            assert!(result.is_err(), "{host}");
        }
        assert!(NetworkPolicy::new(&["api:8080".into()], "secret").is_err());
        assert_eq!(
            network_origin("https://intel.example.test:443/").unwrap(),
            "https://intel.example.test"
        );
        for origin in [
            "null",
            "ftp://intel.example.test",
            "https://secret@intel.example.test",
            "https://intel.example.test/app",
            "https://intel.example.test?token=secret",
        ] {
            let error = network_origin(origin).unwrap_err();
            assert_eq!(error.to_string(), "invalid API origin");
        }
    }

    #[tokio::test]
    async fn ingress_rejects_wrong_hosts_origins_and_bearers_before_dispatch() {
        let access = AccessPolicy {
            port: 8080,
            origins: Arc::new(vec!["http://localhost:3000".into()]),
            network: Some(NetworkPolicy::new(&["api:8080".into()], TOKEN).unwrap()),
        };
        let app = Router::new()
            .route("/api/v1/live", get(|| async { StatusCode::OK }))
            .layer(middleware::from_fn_with_state(access, protect));
        for (method, host, origin, token, status) in [
            (
                "GET",
                "other:8080",
                None,
                Some(TOKEN),
                StatusCode::FORBIDDEN,
            ),
            (
                "GET",
                "api:8080",
                Some("http://other:3000"),
                Some(TOKEN),
                StatusCode::FORBIDDEN,
            ),
            ("GET", "api:8080", None, None, StatusCode::UNAUTHORIZED),
            (
                "GET",
                "api:8080",
                None,
                Some("0123456789abcdefghijklmnopqrstuvwx00"),
                StatusCode::UNAUTHORIZED,
            ),
            ("GET", "API:8080", None, Some(TOKEN), StatusCode::OK),
            (
                "OPTIONS",
                "api:8080",
                Some("http://localhost:3000"),
                None,
                StatusCode::NO_CONTENT,
            ),
        ] {
            let mut request = Request::builder()
                .method(method)
                .uri("/api/v1/live")
                .header(header::HOST, host);
            if let Some(origin) = origin {
                request = request.header(header::ORIGIN, origin);
            }
            if let Some(token) = token {
                request = request.header(header::AUTHORIZATION, format!("Bearer {token}"));
            }
            let response = app
                .clone()
                .oneshot(request.body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), status, "{method} {host} {origin:?}");
            if status == StatusCode::UNAUTHORIZED {
                assert_eq!(response.headers()[header::WWW_AUTHENTICATE], "Bearer");
            }
            if status == StatusCode::NO_CONTENT {
                assert_eq!(
                    response.headers()[header::ACCESS_CONTROL_ALLOW_HEADERS],
                    "authorization, content-type"
                );
            }
        }
    }
}
