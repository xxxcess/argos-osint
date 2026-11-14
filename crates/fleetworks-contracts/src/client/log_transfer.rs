//! Bounded safe-summary exports from the registered log route only.

use reqwest::header;
use reqwest::Method;

use super::Client;
use super::ClientError;
use crate::operational_logs::LogEntry;
use crate::operational_logs::LogQuery;

/// One authorized snapshot page. A continuation means the export is incomplete.
#[derive(Clone, Eq, PartialEq)]
#[non_exhaustive]
pub struct LogExportPage {
    pub bytes: Vec<u8>,
    pub next_cursor: Option<String>,
    pub apps_complete: bool,
}

impl std::fmt::Debug for LogExportPage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LogExportPage")
            .field("byte_count", &self.bytes.len())
            .field("has_continuation", &self.next_cursor.is_some())
            .field("apps_complete", &self.apps_complete)
            .finish()
    }
}

impl Client {
    /// Download one bounded, registered snapshot page of safe log summaries.
    /// Never treat a page with a continuation as a complete export. The caller
    /// owns explicit private-file confirmation and incomplete-export handling.
    pub async fn export_logs(&self, query: &LogQuery) -> Result<LogExportPage, ClientError> {
        query.validate().map_err(|_| ClientError::InvalidRequest)?;
        if self.registration.is_none() {
            return Err(ClientError::InvalidRequest);
        }
        let request = self
            .request(Method::GET, "/api/v1/logs/export")?
            .query(query)
            .build()
            .map_err(|_| ClientError::InvalidRequest)?;
        let mut response = self
            .http
            .execute(request)
            .await
            .map_err(ClientError::transport)?;
        if !response.status().is_success() {
            return Err(ClientError::Status(response.status().as_u16()));
        }
        if response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            != Some("application/x-ndjson")
            || response
                .headers()
                .get(header::CONTENT_DISPOSITION)
                .and_then(|value| value.to_str().ok())
                != Some("attachment; filename=workstation-logs.ndjson")
        {
            return Err(ClientError::InvalidResponse);
        }
        let apps_complete = match response
            .headers()
            .get("x-fleetworks-log-apps-complete")
            .and_then(|value| value.to_str().ok())
        {
            Some("true") => true,
            Some("false") => false,
            _ => return Err(ClientError::InvalidResponse),
        };
        let next_cursor = response
            .headers()
            .get("x-fleetworks-next-log-cursor")
            .map(|value| {
                let value = value.to_str().map_err(|_| ClientError::InvalidResponse)?;
                if !crate::operational_logs::cursor_valid(value) {
                    return Err(ClientError::InvalidResponse);
                }
                Ok(value.to_owned())
            })
            .transpose()?;
        if response
            .content_length()
            .is_some_and(|length| length > super::MAX_RESPONSE_BYTES as u64)
        {
            return Err(ClientError::ResponseTooLarge);
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(ClientError::transport)? {
            if chunk.len() > super::MAX_RESPONSE_BYTES - bytes.len() {
                return Err(ClientError::ResponseTooLarge);
            }
            bytes.extend_from_slice(&chunk);
        }
        let text = std::str::from_utf8(&bytes).map_err(|_| ClientError::InvalidResponse)?;
        let mut count = 0;
        for line in text.lines() {
            count += 1;
            if count > 100 {
                return Err(ClientError::InvalidResponse);
            }
            let row: LogEntry =
                serde_json::from_str(line).map_err(|_| ClientError::InvalidResponse)?;
            row.validate().map_err(|_| ClientError::InvalidResponse)?;
        }
        Ok(LogExportPage {
            bytes,
            next_cursor,
            apps_complete,
        })
    }
}
