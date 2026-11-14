//! Explicit bounded credential-bearing transfer on the configured origin only.

use reqwest::header;
use reqwest::Method;
use serde::Deserialize;
use serde::Serialize;

use super::Client;
use super::ClientError;

const LIMIT: usize = 2 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ConfigurationTransferReceipt {
    pub saved: bool,
    pub configuration_revision: u64,
}

impl Client {
    /// Explicitly download protected schema-4 bytes for a confirmed private file.
    /// The caller must neither log/render these bytes nor overwrite an existing file.
    pub async fn export_configuration(
        &self,
        expected_revision: u64,
    ) -> Result<Vec<u8>, ClientError> {
        let request=self.request(Method::POST,"/api/v1/profile/export")?
            .json(&serde_json::json!({"expected_revision":expected_revision,"include_credentials":true}))
            .build().map_err(|_|ClientError::InvalidRequest)?;
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
            != Some("application/octet-stream")
            || response
                .headers()
                .get(header::CONTENT_DISPOSITION)
                .and_then(|value| value.to_str().ok())
                != Some("attachment; filename=\"fleetworks-configuration.json\"")
        {
            return Err(ClientError::InvalidResponse);
        }
        if response
            .content_length()
            .is_some_and(|length| length > LIMIT as u64)
        {
            return Err(ClientError::ResponseTooLarge);
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(ClientError::transport)? {
            if chunk.len() > LIMIT - bytes.len() {
                return Err(ClientError::ResponseTooLarge);
            }
            bytes.extend_from_slice(&chunk);
        }
        let value: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|_| ClientError::InvalidResponse)?;
        if value
            .get("schema_version")
            .and_then(serde_json::Value::as_u64)
            != Some(4)
        {
            return Err(ClientError::InvalidResponse);
        }
        Ok(bytes)
    }

    /// Replace current-format global configuration after explicit confirmation.
    /// Legacy import, server paths and unbounded/base64 catalog transfer are absent.
    pub async fn import_configuration(
        &self,
        expected_revision: u64,
        bytes: Vec<u8>,
    ) -> Result<ConfigurationTransferReceipt, ClientError> {
        if bytes.is_empty() || bytes.len() > LIMIT {
            return Err(ClientError::RequestTooLarge);
        }
        let value: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|_| ClientError::InvalidRequest)?;
        if value
            .get("schema_version")
            .and_then(serde_json::Value::as_u64)
            != Some(4)
        {
            return Err(ClientError::InvalidRequest);
        }
        let request = self
            .request(Method::POST, "/api/v1/profile/import")?
            .query(&[
                ("expected_revision", expected_revision.to_string()),
                ("replace", "true".to_owned()),
            ])
            .header(header::CONTENT_TYPE, "application/json")
            .body(bytes);
        let receipt: ConfigurationTransferReceipt = Self::send(request).await?;
        if !receipt.saved || receipt.configuration_revision <= expected_revision {
            return Err(ClientError::InvalidResponse);
        }
        Ok(receipt)
    }
}
