//! Durable chronological event contract, secret redaction, and stream parts.

use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::contracts::InvestigationSurface;

/// One durable event in an investigation's chronological trace.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct InvestigationEvent {
    pub id: String,
    pub investigation_id: String,
    pub sequence: i64,
    pub occurrence_time: String,
    pub recording_time: String,
    pub surface: InvestigationSurface,
    pub run_id: String,
    pub directive_id: String,
    pub task_id: String,
    pub revision: i64,
    pub parent_event_id: Option<String>,
    pub role: String,
    pub model: String,
    pub provider: String,
    pub attempt_id: String,
    pub event_type: String,
    pub status: String,
    pub summary: String,
    pub payload: Value,
    pub evidence_refs: Vec<String>,
    pub superseded_event_id: Option<String>,
}

impl InvestigationEvent {
    pub fn new(
        investigation_id: impl Into<String>,
        sequence: i64,
        surface: InvestigationSurface,
        event_type: impl Into<String>,
        summary: impl Into<String>,
    ) -> Self {
        let ts = Utc::now().to_rfc3339();
        let inv_id = investigation_id.into();
        let ev_type = event_type.into();
        let id = format!(
            "ev:{}:{}:{}",
            inv_id,
            sequence,
            Utc::now().timestamp_subsec_millis()
        );

        Self {
            id,
            investigation_id: inv_id,
            sequence,
            occurrence_time: ts.clone(),
            recording_time: ts,
            surface,
            run_id: String::new(),
            directive_id: String::new(),
            task_id: String::new(),
            revision: 1,
            parent_event_id: None,
            role: String::new(),
            model: String::new(),
            provider: String::new(),
            attempt_id: String::new(),
            event_type: ev_type,
            status: "ok".into(),
            summary: summary.into(),
            payload: serde_json::json!({}),
            evidence_refs: Vec::new(),
            superseded_event_id: None,
        }
    }

    pub fn with_role(
        mut self,
        role: impl Into<String>,
        provider: impl Into<String>,
        model: impl Into<String>,
    ) -> Self {
        self.role = role.into();
        self.provider = provider.into();
        self.model = model.into();
        self
    }

    pub fn with_task(
        mut self,
        run_id: impl Into<String>,
        directive_id: impl Into<String>,
        task_id: impl Into<String>,
        revision: i64,
    ) -> Self {
        self.run_id = run_id.into();
        self.directive_id = directive_id.into();
        self.task_id = task_id.into();
        self.revision = revision;
        self
    }

    pub fn with_payload(mut self, payload: Value) -> Self {
        let mut p = payload;
        redact_secrets(&mut p);
        self.payload = p;
        self
    }
}

/// Redact credentials, API keys, tokens, and authorization headers from JSON values.
pub fn redact_secrets(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for (k, v) in map.iter_mut() {
                let lower = k.to_ascii_lowercase();
                if lower.contains("key")
                    || lower.contains("token")
                    || lower.contains("secret")
                    || lower.contains("auth")
                    || lower.contains("password")
                    || lower.contains("credential")
                {
                    if let Value::String(_) = v {
                        *v = Value::String("[REDACTED]".into());
                    }
                } else {
                    redact_secrets(v);
                }
            }
        }
        Value::Array(arr) => {
            for item in arr {
                redact_secrets(item);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn redacts_sensitive_keys() {
        let mut v = json!({
            "api_key": "sk-1234567890abcdef",
            "nested": {
                "bearer_token": "secret_jwt",
                "normal_field": "public_data"
            }
        });
        redact_secrets(&mut v);
        assert_eq!(v["api_key"], "[REDACTED]");
        assert_eq!(v["nested"]["bearer_token"], "[REDACTED]");
        assert_eq!(v["nested"]["normal_field"], "public_data");
    }
}
