//! Shared bounded HTTP transport for allowlisted private service routes.

use std::time::Duration;

use anyhow::ensure;
use axum::body::Bytes;
use axum::extract::{FromRequest, Request};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use url::Url;

use crate::{network, ApiError};

const MAX_QUERY_BYTES: usize = 8 * 1024;
const MAX_RESPONSE_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Copy, Debug)]
pub(crate) enum Service {
    Brain,
    Tools,
}

impl Service {
    fn oversized_query(self) -> &'static str {
        match self {
            Self::Brain => "Brain query exceeds the size limit",
            Self::Tools => "Tools query exceeds the size limit",
        }
    }

    fn unavailable(self, reason: &'static str) -> Response {
        eprintln!("{self:?} service relay failed: {reason}");
        let message = match self {
            Self::Brain => "Brain service unavailable",
            Self::Tools => "Tools service unavailable",
        };
        ApiError(StatusCode::BAD_GATEWAY, message).into_response()
    }
}

#[derive(Clone)]
pub(crate) struct PrivateClient {
    origin: Url,
    authorization: HeaderValue,
    http: reqwest::Client,
    service: Service,
}

impl PrivateClient {
    pub(crate) fn new(origin: &str, token: &str, service: Service) -> anyhow::Result<Self> {
        let origin = network::validate_origin(origin)?;
        ensure!(
            network::valid_token(token.as_bytes()),
            "invalid {service:?} bearer token"
        );
        let mut authorization = HeaderValue::from_str(&format!("Bearer {token}"))?;
        authorization.set_sensitive(true);
        let http = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(50))
            .build()?;
        Ok(Self {
            origin,
            authorization,
            http,
            service,
        })
    }

    pub(crate) async fn forward(self, request: Request) -> Result<Response, Response> {
        // The service router selects the operation. Request data cannot
        // change the upstream origin.
        let uri = request.uri();
        if uri
            .query()
            .is_some_and(|query| query.len() > MAX_QUERY_BYTES)
        {
            return Err(
                ApiError(StatusCode::URI_TOO_LONG, self.service.oversized_query()).into_response(),
            );
        }
        let mut url = self.origin;
        url.set_path(uri.path());
        url.set_query(uri.query());
        let method = request.method().clone();
        let content_type = request.headers().get(header::CONTENT_TYPE).cloned();
        let payload = Bytes::from_request(request, &())
            .await
            .map_err(IntoResponse::into_response)?;
        // Public Authorization, Host, Origin and cookies never leave the gateway.
        let mut upstream = self
            .http
            .request(method, url)
            .header(header::AUTHORIZATION, self.authorization)
            .body(payload);
        if let Some(value) = content_type {
            upstream = upstream.header(header::CONTENT_TYPE, value);
        }
        let mut upstream = upstream
            .send()
            .await
            .map_err(|_| self.service.unavailable("request failed"))?;
        let status = upstream.status();
        if status.is_redirection()
            || matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN)
        {
            return Err(self
                .service
                .unavailable("service refused the internal request"));
        }
        let content_type = upstream.headers().get(header::CONTENT_TYPE).cloned();
        let mut bytes = Vec::new();
        while let Some(chunk) = upstream
            .chunk()
            .await
            .map_err(|_| self.service.unavailable("response interrupted"))?
        {
            if chunk.len() > MAX_RESPONSE_BYTES - bytes.len() {
                return Err(self.service.unavailable("response exceeds the size limit"));
            }
            bytes.extend_from_slice(&chunk);
        }
        let mut response = (status, bytes).into_response();
        if let Some(value) = content_type {
            response.headers_mut().insert(header::CONTENT_TYPE, value);
        }
        Ok(response)
    }
}
