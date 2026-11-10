use std::fmt;
use std::time::Duration;

use reqwest::header::{self, HeaderValue};
use reqwest::{Method, RequestBuilder};
use serde::de::DeserializeOwned;
use serde::Serialize;
use url::Url;

use crate::{Ready, API_VERSION, READY_PATH};

const MAX_REQUEST_BYTES: usize = 16 * 1024;
const MAX_QUERY_BYTES: usize = 8 * 1024;
const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
const MAX_TOKEN_BYTES: usize = 4 * 1024;

/// Client failures contain neither the API's prose nor request URLs or credentials.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ClientError {
    InvalidOrigin,
    MissingToken,
    InvalidToken,
    InvalidRoute,
    InvalidRequest,
    RequestTooLarge,
    Transport,
    Timeout,
    Status(u16),
    ResponseTooLarge,
    InvalidResponse,
    UnsupportedVersion(u16),
}

impl fmt::Display for ClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::InvalidOrigin => {
                write!(f, "expected a loopback HTTP or authenticated HTTPS origin")
            }
            Self::MissingToken => write!(f, "a bearer token is required for a remote API origin"),
            Self::InvalidToken => write!(f, "invalid API bearer token"),
            Self::InvalidRoute => write!(f, "invalid versioned API route"),
            Self::InvalidRequest => write!(f, "invalid API request"),
            Self::RequestTooLarge => write!(f, "API request exceeds the size limit"),
            Self::Transport => write!(f, "could not reach the configured API"),
            Self::Timeout => write!(f, "API request timed out"),
            Self::Status(status) => write!(f, "API returned HTTP {status}"),
            Self::ResponseTooLarge => write!(f, "API response exceeds the size limit"),
            Self::InvalidResponse => write!(f, "invalid API response"),
            Self::UnsupportedVersion(version) => write!(f, "unsupported API version {version}"),
        }
    }
}

impl std::error::Error for ClientError {}

impl ClientError {
    fn transport(error: reqwest::Error) -> Self {
        if error.is_timeout() {
            Self::Timeout
        } else {
            Self::Transport
        }
    }
}

/// A client pinned to one public API origin.
///
/// Authorization stays on that origin. Redirects and environment proxies are disabled.
#[derive(Clone)]
pub struct Client {
    origin: Url,
    token: Option<HeaderValue>,
    http: reqwest::Client,
}

impl Client {
    /// Connect to an HTTP loopback origin or an HTTPS origin.
    ///
    /// Unauthenticated access is restricted to loopback for legacy Argos compatibility.
    pub fn new(origin: &str, token: Option<&str>) -> Result<Self, ClientError> {
        let origin = Url::parse(origin).map_err(|_| ClientError::InvalidOrigin)?;
        let loopback = match origin.host() {
            Some(url::Host::Domain(name)) => name == "localhost",
            Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
            Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
            None => false,
        };
        if origin.host().is_none()
            || !matches!(origin.scheme(), "http" | "https")
            || (origin.scheme() == "http" && !loopback)
            || !origin.username().is_empty()
            || origin.password().is_some()
            || origin.path() != "/"
            || origin.query().is_some()
            || origin.fragment().is_some()
        {
            return Err(ClientError::InvalidOrigin);
        }
        if !loopback && token.is_none() {
            return Err(ClientError::MissingToken);
        }
        let token = token.map(bearer_header).transpose()?;
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .connect_timeout(Duration::from_secs(5))
            // The existing API's own deadline is 45 seconds.
            .timeout(Duration::from_secs(50))
            .build()
            .map_err(ClientError::transport)?;
        Ok(Self {
            origin,
            token,
            http,
        })
    }

    /// Read readiness and verify the reported API version.
    pub async fn ready(&self) -> Result<Ready, ClientError> {
        let ready: Ready = self.get(READY_PATH).await?;
        if ready.api_version != API_VERSION {
            return Err(ClientError::UnsupportedVersion(ready.api_version));
        }
        Ok(ready)
    }

    /// Send an HTTP GET to a literal versioned route.
    pub async fn get<T: DeserializeOwned>(&self, route: &str) -> Result<T, ClientError> {
        Self::send(self.request(Method::GET, route)?).await
    }

    /// Send an HTTP GET with encoded query fields.
    pub async fn get_with_query<T: DeserializeOwned>(
        &self,
        route: &str,
        query: &(impl Serialize + ?Sized),
    ) -> Result<T, ClientError> {
        Self::send(self.request(Method::GET, route)?.query(query)).await
    }

    /// Send JSON to a literal versioned route.
    pub async fn post<T: DeserializeOwned>(
        &self,
        route: &str,
        body: &(impl Serialize + ?Sized),
    ) -> Result<T, ClientError> {
        let request = self.request(Method::POST, route)?;
        let body = serde_json::to_vec(body).map_err(|_| ClientError::InvalidRequest)?;
        if body.len() > MAX_REQUEST_BYTES {
            return Err(ClientError::RequestTooLarge);
        }
        Self::send(
            request
                .header(header::CONTENT_TYPE, "application/json")
                .body(body),
        )
        .await
    }

    fn request(&self, method: Method, route: &str) -> Result<RequestBuilder, ClientError> {
        let tail = route
            .strip_prefix("/api/v1/")
            .ok_or(ClientError::InvalidRoute)?;
        if route.len() > 128
            || !tail.split('/').all(|segment| {
                !segment.is_empty()
                    && segment.bytes().all(|byte| {
                        byte.is_ascii_lowercase()
                            || byte.is_ascii_digit()
                            || matches!(byte, b'-' | b'_')
                    })
            })
        {
            return Err(ClientError::InvalidRoute);
        }
        let mut url = self.origin.clone();
        url.set_path(route);
        let request = self
            .http
            .request(method, url)
            .header(header::ACCEPT, "application/json");
        Ok(match &self.token {
            Some(token) => request.header(header::AUTHORIZATION, token.clone()),
            None => request,
        })
    }

    async fn send<T: DeserializeOwned>(request: RequestBuilder) -> Result<T, ClientError> {
        let (http, request) = request.build_split();
        let request = request.map_err(|_| ClientError::InvalidRequest)?;
        if request
            .url()
            .query()
            .is_some_and(|query| query.len() > MAX_QUERY_BYTES)
        {
            return Err(ClientError::RequestTooLarge);
        }
        let mut response = http
            .execute(request)
            .await
            .map_err(ClientError::transport)?;
        if !response.status().is_success() {
            return Err(ClientError::Status(response.status().as_u16()));
        }
        if response
            .content_length()
            .is_some_and(|len| len > MAX_RESPONSE_BYTES as u64)
        {
            return Err(ClientError::ResponseTooLarge);
        }
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(ClientError::transport)? {
            if chunk.len() > MAX_RESPONSE_BYTES - body.len() {
                return Err(ClientError::ResponseTooLarge);
            }
            body.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&body).map_err(|_| ClientError::InvalidResponse)
    }
}

fn bearer_header(token: &str) -> Result<HeaderValue, ClientError> {
    let value = token.trim_end_matches('=');
    if token.len() > MAX_TOKEN_BYTES
        || value.is_empty()
        || !value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~' | b'+' | b'/')
        })
    {
        return Err(ClientError::InvalidToken);
    }
    let mut header =
        HeaderValue::from_str(&format!("Bearer {token}")).map_err(|_| ClientError::InvalidToken)?;
    header.set_sensitive(true);
    Ok(header)
}

#[cfg(test)]
mod tests;
