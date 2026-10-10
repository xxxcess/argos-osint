//! Portable profile configuration: the schema-v1 document, its export, import,
//! validation, and the recoverable three-file commit that publishes it.
//!
//! The portable contract is [`ProfileConfig`]; the JSON Schema is
//! `docs/schemas/profile-config-v1.schema.json`. `SettingsFile` is never exposed
//! wholesale: only the documented fields travel, so unrelated settings, recon
//! limits and local state cannot be overwritten by a foreign document.
//!
//! A document is validated **completely** before any write. Validation walks the
//! parsed JSON itself so every error carries a JSON pointer, rejects duplicate
//! object keys (a `serde_json::Value` silently keeps the last of a repeated key,
//! which is exactly how a stale value wins without being seen), and never echoes
//! key material: an error names a field path and an expected shape, never a
//! value.
//!
//! Import is a **merge by stable id**: provider, tool, role and quota-group ids
//! are the join keys. Values explicitly supplied replace the local value; an
//! explicit null clears it back to documented inheritance; an absent field
//! leaves the local value untouched. The result is published by
//! [`commit_profile_config`], which takes [`crate::config_commit::ConfigLock`]
//! and lands `config.toml`, `auth.json` and `quota.json` as one all-or-nothing
//! revision.

use std::collections::HashSet;
use std::env;
use std::fmt;
use std::marker::PhantomData;
use std::path::Path;

use anyhow::{bail, Context, Result};
use serde::de::value::{MapAccessDeserializer, SeqAccessDeserializer};
use serde::de::{self, Deserializer, MapAccess, SeqAccess, Visitor};
use serde::ser::Serializer;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::config_commit::{CommitFile, ConfigLock};
use crate::paths;
use crate::provider::{self, ModelAssignment, ModelRoute, SettingsFile};
use crate::provider_metrics::QuotaSetting;
use crate::secrets::{AuthFile, ProviderSecret};

/// Portable configuration schema version. Argos rejects any other value.
pub const SCHEMA_VERSION: u32 = 1;
/// A document larger than this is refused before it is parsed.
pub const MAX_DOCUMENT_BYTES: usize = 1 << 20;

/// Provider kinds the portable document accepts.
pub const PROVIDER_KINDS: &[&str] = &[
    "openrouter",
    "google",
    "nvidia",
    "grok",
    "openai",
    "local",
    "subscription",
];
/// Where a verified quota number came from.
pub const QUOTA_SOURCES: &[&str] = &["provider_docs", "probe", "desk_assumption", "unset"];
/// The keyed OSINT tool providers, in `provider::KEY_ENV` order. Holehe is
/// keyless and never gains a fabricated credential field.
pub const TOOL_CREDENTIAL_PROVIDERS: &[&str] = &[
    "firecrawl",
    "hunter",
    "sociavault",
    "newsapi",
    "courtlistener",
    "gnews",
    "newsdata",
    "currents",
    "whoxy",
];
/// Roles that serve general synthesis. A decisions (Jev) model cannot serve
/// them, so the document is rejected rather than routed into a transport that
/// silently degrades.
pub const GENERAL_ROLES: &[&str] = &[
    "recon",
    "synthesis",
    "classifier",
    "summarization",
    "evidence_curator",
    "entity_resolver",
    "claim_assessor",
    "investigation_controller",
];
/// Roles that serve typed decisions, and therefore need a decisions model.
pub const DECISION_ROLES: &[&str] = &["tool_picker", "decision_model", "decision_fallback"];

/// Root fields a document must carry.
const ROOT_REQUIRED: &[&str] = &[
    "schema_version",
    "providers",
    "tool_credentials",
    "model_roles",
    "rate_limits",
];
/// Every root field, required or optional.
const ROOT_FIELDS: &[&str] = &[
    "schema_version",
    "exported_at",
    "providers",
    "tool_credentials",
    "model_roles",
    "rate_limits",
];
const PROVIDER_FIELDS: &[&str] = &[
    "id",
    "kind",
    "base_url",
    "default_model",
    "credential",
    "quota_group_id",
];
const CREDENTIAL_FIELDS: &[&str] = &["source", "api_key", "name"];
const TOOL_FIELDS: &[&str] = &["provider", "primary", "fallback"];
const ROLE_ASSIGNMENT_FIELDS: &[&str] = &["primary", "fallbacks"];
const ROUTE_FIELDS: &[&str] = &["provider_id", "model"];
const QUOTA_FIELDS: &[&str] = &[
    "quota_group_id",
    "scope",
    "verified_rpm",
    "verified_tpm",
    "verified_rpd",
    "local_rpm",
    "concurrency",
    "source",
    "verified_at",
];
/// Every role key a `model_roles` object must carry.
const ROLE_KEYS: &[&str] = &[
    "recon",
    "synthesis",
    "tool_picker",
    "classifier",
    "summarization",
    "evidence_curator",
    "entity_resolver",
    "claim_assessor",
    "investigation_controller",
    "decision_model",
];
/// Every role key a `model_roles` object may carry.
const ROLE_FIELDS: &[&str] = &[
    "recon",
    "synthesis",
    "tool_picker",
    "classifier",
    "summarization",
    "evidence_curator",
    "entity_resolver",
    "claim_assessor",
    "investigation_controller",
    "decision_model",
    "decision_fallback",
];

/// One validation problem: a JSON pointer and a message that never carries a
/// value from the document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigIssue {
    pub pointer: String,
    pub message: String,
}

impl ConfigIssue {
    pub fn new(pointer: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            pointer: pointer.into(),
            message: message.into(),
        }
    }
}

impl fmt::Display for ConfigIssue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.pointer.is_empty() {
            f.write_str(&self.message)
        } else {
            write!(f, "{}: {}", self.pointer, self.message)
        }
    }
}

/// One entry of the redacted change summary shown before the Import button is
/// armed. A summary names fields and ids, never a credential value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigChange {
    pub area: String,
    pub id: String,
    pub summary: String,
}

/// How a credential travels in the portable document.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CredentialSource {
    /// A saved key copied in the document.
    #[default]
    Inline,
    /// The name of an environment variable resolved at runtime.
    Env,
    /// A keyless route.
    None,
}

impl CredentialSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Inline => "inline",
            Self::Env => "env",
            Self::None => "none",
        }
    }
}

/// A tagged credential: `{source:"inline", api_key}`, `{source:"env", name}` or
/// `{source:"none"}`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Credential {
    #[serde(rename = "source")]
    pub source: CredentialSource,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub api_key: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
}

impl Credential {
    /// A credential carrying a saved key.
    pub fn inline(api_key: impl Into<String>) -> Self {
        Self {
            source: CredentialSource::Inline,
            api_key: api_key.into(),
            name: String::new(),
        }
    }

    /// A credential that names an environment variable.
    pub fn env(name: impl Into<String>) -> Self {
        Self {
            source: CredentialSource::Env,
            api_key: String::new(),
            name: name.into(),
        }
    }

    /// A keyless route.
    pub fn none() -> Self {
        Self {
            source: CredentialSource::None,
            api_key: String::new(),
            name: String::new(),
        }
    }
}

/// One configured model account.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderEntry {
    pub id: String,
    pub kind: String,
    pub base_url: String,
    pub default_model: String,
    pub credential: Credential,
    #[serde(default)]
    pub quota_group_id: String,
}

/// How a mergeable field was mentioned by a document.
///
/// Import replaces only what the document explicitly supplies, so a field the
/// document never mentions is different from one it clears with an explicit
/// null: the first preserves the local value, the second removes it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Supplied<T> {
    /// The document never mentioned the field.
    #[default]
    Absent,
    /// The document carried an explicit null: remove the local value.
    Removed,
    /// The document supplied a value.
    Value(T),
}

impl<T> Supplied<T> {
    pub fn is_absent(&self) -> bool {
        matches!(self, Self::Absent)
    }

    pub fn value(&self) -> Option<&T> {
        match self {
            Self::Value(value) => Some(value),
            _ => None,
        }
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Supplied<T> {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct SuppliedVisitor<T>(PhantomData<T>);

        impl<'de, T: Deserialize<'de>> Visitor<'de> for SuppliedVisitor<T> {
            type Value = Supplied<T>;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a value or null")
            }

            fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
                Ok(Supplied::Removed)
            }

            fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
                Ok(Supplied::Removed)
            }

            fn visit_some<D: Deserializer<'de>>(
                self,
                deserializer: D,
            ) -> Result<Self::Value, D::Error> {
                T::deserialize(deserializer).map(Supplied::Value)
            }

            fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Self::Value, A::Error> {
                T::deserialize(MapAccessDeserializer::new(map)).map(Supplied::Value)
            }

            fn visit_seq<A: SeqAccess<'de>>(self, seq: A) -> Result<Self::Value, A::Error> {
                T::deserialize(SeqAccessDeserializer::new(seq)).map(Supplied::Value)
            }
        }

        deserializer.deserialize_option(SuppliedVisitor(PhantomData))
    }
}

impl<T: Serialize> Serialize for Supplied<T> {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::Absent | Self::Removed => serializer.serialize_none(),
            Self::Value(value) => value.serialize(serializer),
        }
    }
}

/// Primary and fallback credentials for one keyed OSINT tool provider.
///
/// Both slots carry the same rule: a field the document never mentions keeps the
/// local value, an explicit null removes it (`no account configured`), and a
/// credential replaces it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolCredentialEntry {
    pub provider: String,
    #[serde(default, skip_serializing_if = "Supplied::is_absent")]
    pub primary: Supplied<Credential>,
    #[serde(default, skip_serializing_if = "Supplied::is_absent")]
    pub fallback: Supplied<Credential>,
}

/// One inference route inside a role.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Route {
    pub provider_id: String,
    pub model: String,
}

/// One role assignment. `primary: null` preserves documented inheritance;
/// `fallbacks` only changes when the document supplies an array.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoleAssignment {
    pub primary: Option<Route>,
    #[serde(default, skip_serializing_if = "Supplied::is_absent")]
    pub fallbacks: Supplied<Vec<Route>>,
}

/// Every documented role, including the decision roles and the optional
/// decision fallback.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoleDocument {
    pub recon: RoleAssignment,
    pub synthesis: RoleAssignment,
    pub tool_picker: RoleAssignment,
    pub classifier: RoleAssignment,
    pub summarization: RoleAssignment,
    pub evidence_curator: RoleAssignment,
    pub entity_resolver: RoleAssignment,
    pub claim_assessor: RoleAssignment,
    pub investigation_controller: RoleAssignment,
    pub decision_model: RoleAssignment,
    #[serde(default)]
    pub decision_fallback: Option<RoleAssignment>,
}

impl RoleDocument {
    /// Every role key with its assignment, in documentation order.
    pub fn entries(&self) -> Vec<(&'static str, &RoleAssignment)> {
        vec![
            ("recon", &self.recon),
            ("synthesis", &self.synthesis),
            ("tool_picker", &self.tool_picker),
            ("classifier", &self.classifier),
            ("summarization", &self.summarization),
            ("evidence_curator", &self.evidence_curator),
            ("entity_resolver", &self.entity_resolver),
            ("claim_assessor", &self.claim_assessor),
            ("investigation_controller", &self.investigation_controller),
            ("decision_model", &self.decision_model),
        ]
    }
}

/// The portable profile configuration document.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileConfig {
    pub schema_version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exported_at: Option<String>,
    pub providers: Vec<ProviderEntry>,
    pub tool_credentials: Vec<ToolCredentialEntry>,
    pub model_roles: RoleDocument,
    pub rate_limits: Vec<QuotaSetting>,
}

/// A validated document plus everything the Import screen shows before the user
/// commits: blocking-independent warnings and the redacted change summary.
#[derive(Clone, Debug, Default)]
pub struct ImportPlan {
    pub config: ProfileConfig,
    pub warnings: Vec<ConfigIssue>,
    pub changes: Vec<ConfigChange>,
}

impl ImportPlan {
    /// The redacted change summary against the live configuration. Fields the
    /// document does not mention never appear here.
    pub fn changes_against(
        &self,
        settings: &SettingsFile,
        auth: &AuthFile,
        quotas: &QuotaSettingsFile,
    ) -> Vec<ConfigChange> {
        let before = ConfigurationSnapshot {
            settings: settings.clone(),
            auth: auth.clone(),
            quotas: quotas.clone(),
        };
        let mut after = before.clone();
        if after.apply(self).is_err() {
            return Vec::new();
        }
        describe_changes(&self.config, &before, &after)
    }
}

/// The third configuration file: portable quota settings, committed with
/// `config.toml` and `auth.json` under one lock.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct QuotaSettingsFile {
    #[serde(default)]
    pub settings: Vec<QuotaSetting>,
}

impl QuotaSettingsFile {
    pub fn load() -> Result<Self> {
        Self::load_from(&paths::quota_path())
    }

    pub fn load_from(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw =
            std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
        if raw.trim().is_empty() {
            return Ok(Self::default());
        }
        serde_json::from_str(&raw).with_context(|| format!("parse {}", path.display()))
    }

    /// Commits only the `quota` slot, serialised behind the app-wide lock.
    pub fn save(&self) -> Result<()> {
        paths::ensure_home()?;
        let lock = ConfigLock::acquire()?;
        lock.commit_files(&[CommitFile::new(
            "quota",
            paths::quota_path(),
            serde_json::to_string_pretty(self)?,
        )])?;
        Ok(())
    }

    /// Secure single-file write for non-canonical/test paths.
    pub fn save_to(&self, path: &Path) -> Result<()> {
        crate::config_commit::write_secure(path, &serde_json::to_string_pretty(self)?)
    }

    /// Inserts or replaces the setting with the same `(quota_group_id, scope)`.
    pub fn upsert(&mut self, setting: QuotaSetting) {
        let found = self
            .settings
            .iter_mut()
            .find(|existing| merge_key(existing) == merge_key(&setting));
        match found {
            Some(existing) => *existing = setting,
            None => self.settings.push(setting),
        }
    }

    /// Every setting for one quota group, in stored order.
    pub fn group(&self, quota_group_id: &str) -> Vec<&QuotaSetting> {
        self.settings
            .iter()
            .filter(|setting| setting.quota_group_id == quota_group_id)
            .collect()
    }

    /// True when no setting is configured at all.
    pub fn is_empty(&self) -> bool {
        self.settings.is_empty()
    }
}

/// The merge key a quota setting is joined on. An empty scope means the whole
/// account, serialised as `*`.
pub fn merge_key(setting: &QuotaSetting) -> (String, String) {
    let scope = if setting.scope.trim().is_empty() {
        "*".to_string()
    } else {
        setting.scope.trim().to_string()
    };
    (setting.quota_group_id.trim().to_string(), scope)
}

/// Parses JSON while rejecting a duplicate key anywhere in the document.
///
/// `serde_json::Map` keeps the last of a repeated key, so a document could
/// override `schema_version` after the fact and still look well formed. This
/// deserializer recurses with itself for every nested value, so no object in the
/// document escapes the check.
struct NoDuplicates(Value);

impl<'de> Deserialize<'de> for NoDuplicates {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct ValueVisitor;

        impl<'de> Visitor<'de> for ValueVisitor {
            type Value = Value;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("any JSON value")
            }

            fn visit_bool<E: de::Error>(self, value: bool) -> Result<Value, E> {
                Ok(Value::Bool(value))
            }

            fn visit_i64<E: de::Error>(self, value: i64) -> Result<Value, E> {
                Ok(Value::from(value))
            }

            fn visit_u64<E: de::Error>(self, value: u64) -> Result<Value, E> {
                Ok(Value::from(value))
            }

            fn visit_f64<E: de::Error>(self, value: f64) -> Result<Value, E> {
                serde_json::Number::from_f64(value)
                    .map(Value::Number)
                    .ok_or_else(|| de::Error::custom("a JSON number must be finite"))
            }

            fn visit_str<E: de::Error>(self, value: &str) -> Result<Value, E> {
                Ok(Value::String(value.to_string()))
            }

            fn visit_string<E: de::Error>(self, value: String) -> Result<Value, E> {
                Ok(Value::String(value))
            }

            fn visit_unit<E: de::Error>(self) -> Result<Value, E> {
                Ok(Value::Null)
            }

            fn visit_none<E: de::Error>(self) -> Result<Value, E> {
                Ok(Value::Null)
            }

            fn visit_some<D: Deserializer<'de>>(self, deserializer: D) -> Result<Value, D::Error> {
                deserializer.deserialize_any(ValueVisitor)
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Value, A::Error> {
                let mut seen: HashSet<String> = HashSet::new();
                let mut out = Map::new();
                while let Some(key) = map.next_key::<String>()? {
                    if !seen.insert(key.clone()) {
                        return Err(de::Error::custom(format!("duplicate object key \"{key}\"")));
                    }
                    let value = map.next_value::<NoDuplicates>()?.0;
                    out.insert(key, value);
                }
                Ok(Value::Object(out))
            }

            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Value, A::Error> {
                let mut out = Vec::new();
                while let Some(item) = seq.next_element::<NoDuplicates>()? {
                    out.push(item.0);
                }
                Ok(Value::Array(out))
            }
        }

        deserializer.deserialize_any(ValueVisitor).map(NoDuplicates)
    }
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

fn issue(pointer: &str, message: impl Into<String>) -> ConfigIssue {
    ConfigIssue::new(pointer, message)
}

fn child(pointer: &str, key: &str) -> String {
    if pointer.is_empty() {
        format!("/{key}")
    } else {
        format!("{pointer}/{key}")
    }
}

fn entry(pointer: &str, index: usize) -> String {
    format!("{pointer}/{index}")
}

fn check_known_fields(
    obj: &Map<String, Value>,
    allowed: &[&str],
    pointer: &str,
    issues: &mut Vec<ConfigIssue>,
) {
    for key in obj.keys() {
        if !allowed.contains(&key.as_str()) {
            issues.push(issue(&child(pointer, key), "unknown field"));
        }
    }
}

fn check_required(
    obj: &Map<String, Value>,
    required: &[&str],
    pointer: &str,
    issues: &mut Vec<ConfigIssue>,
) {
    for key in required {
        if !obj.contains_key(*key) {
            issues.push(issue(&child(pointer, key), "required field is missing"));
        }
    }
}

/// A string field that must be present and must not be null.
fn required_string<'a>(
    obj: &'a Map<String, Value>,
    key: &str,
    pointer: &str,
    issues: &mut Vec<ConfigIssue>,
) -> Option<&'a str> {
    match obj.get(key) {
        None => None,
        Some(Value::String(text)) => Some(text.as_str()),
        Some(_) => {
            issues.push(issue(&child(pointer, key), "must be a string"));
            None
        }
    }
}

/// A string field where null and absence both mean "not supplied".
fn optional_string<'a>(
    obj: &'a Map<String, Value>,
    key: &str,
    pointer: &str,
    issues: &mut Vec<ConfigIssue>,
) -> Option<&'a str> {
    match obj.get(key) {
        None | Some(Value::Null) => None,
        Some(Value::String(text)) => Some(text.as_str()),
        Some(_) => {
            issues.push(issue(&child(pointer, key), "must be a string"));
            None
        }
    }
}

fn nonempty<'a>(
    value: Option<&'a str>,
    pointer: &str,
    issues: &mut Vec<ConfigIssue>,
) -> Option<&'a str> {
    match value {
        Some(text) if !text.trim().is_empty() => Some(text),
        Some(_) => {
            issues.push(issue(pointer, "must not be empty"));
            None
        }
        None => None,
    }
}

fn object_value<'a>(
    value: Option<&'a Value>,
    pointer: &str,
    issues: &mut Vec<ConfigIssue>,
) -> Option<&'a Map<String, Value>> {
    match value {
        None | Some(Value::Null) => None,
        Some(Value::Object(map)) => Some(map),
        Some(_) => {
            issues.push(issue(pointer, "must be an object"));
            None
        }
    }
}

fn array_value<'a>(
    value: Option<&'a Value>,
    pointer: &str,
    issues: &mut Vec<ConfigIssue>,
) -> Option<&'a Vec<Value>> {
    match value {
        None | Some(Value::Null) => None,
        Some(Value::Array(items)) => Some(items),
        Some(_) => {
            issues.push(issue(pointer, "must be an array"));
            None
        }
    }
}

fn is_env_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) if first.is_ascii_alphabetic() || first == '_' => {}
        _ => return false,
    }
    chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

/// Validates a positive integer. `zero_allowed` is the explicit-zero case the
/// schema reserves for a disabled local override.
fn check_limit(
    obj: &Map<String, Value>,
    key: &str,
    pointer: &str,
    zero_allowed: bool,
    maximum: Option<u64>,
    issues: &mut Vec<ConfigIssue>,
) {
    let Some(value) = obj.get(key) else {
        return;
    };
    let here = &child(pointer, key);
    let Some(number) = value.as_f64() else {
        issues.push(issue(here, "must be an integer"));
        return;
    };
    if !number.is_finite() {
        issues.push(issue(here, "must be a finite integer"));
        return;
    }
    if number.fract() != 0.0 {
        issues.push(issue(here, "must be an integer"));
        return;
    }
    let number = number as u64;
    if number == 0 && !zero_allowed {
        issues.push(issue(here, "must be greater than zero"));
        return;
    }
    if let Some(max) = maximum {
        if number > max {
            issues.push(issue(here, format!("must be at most {max}")));
        }
    }
}

fn check_scheme(url: &str, kind: &str, pointer: &str, issues: &mut Vec<ConfigIssue>) {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        issues.push(issue(pointer, "must not be empty"));
        return;
    }
    match url::Url::parse(trimmed) {
        Err(_) => issues.push(issue(pointer, "must be an absolute URL")),
        Ok(parsed) => {
            if parsed.host_str().unwrap_or_default().is_empty() {
                issues.push(issue(pointer, "must include a host"));
                return;
            }
            let allowed = kind == "local" || parsed.scheme() == "https";
            if !allowed {
                issues.push(issue(
                    pointer,
                    "must use https unless the kind is a documented local route",
                ));
            }
        }
    }
}

/// Validates a credential object and returns its source.
fn validate_credential(
    credential: &Map<String, Value>,
    pointer: &str,
    warnings: &mut Vec<ConfigIssue>,
    issues: &mut Vec<ConfigIssue>,
) -> Option<CredentialSource> {
    check_known_fields(credential, CREDENTIAL_FIELDS, pointer, issues);
    check_required(credential, &["source"], pointer, issues);
    let source = required_string(credential, "source", pointer, issues)?;
    let source = match source.trim() {
        "inline" => CredentialSource::Inline,
        "env" => CredentialSource::Env,
        "none" => CredentialSource::None,
        other => {
            issues.push(issue(
                &child(pointer, "source"),
                format!("must be one of inline, env, none; found {other}"),
            ));
            return None;
        }
    };
    match source {
        CredentialSource::Inline => {
            if let Some(key) = required_string(credential, "api_key", pointer, issues) {
                if key.trim().is_empty() {
                    issues.push(issue(
                        &child(pointer, "api_key"),
                        "a blank key is not a masked placeholder",
                    ));
                }
            }
            if let Some(name) = optional_string(credential, "name", pointer, issues) {
                if !name.trim().is_empty() {
                    issues.push(issue(
                        &child(pointer, "name"),
                        "an inline credential must not name an environment variable",
                    ));
                }
            }
        }
        CredentialSource::Env => {
            let name = nonempty(
                required_string(credential, "name", pointer, issues),
                &child(pointer, "name"),
                issues,
            );
            if let Some(name) = name {
                if !is_env_name(name) {
                    issues.push(issue(
                        &child(pointer, "name"),
                        "must be an environment variable name",
                    ));
                } else if env::var(name).is_err() {
                    warnings.push(issue(
                        &child(pointer, "name"),
                        format!(
                            "the environment variable {name} is not set here: the route stays unusable until it resolves"
                        ),
                    ));
                }
            }
            if let Some(key) = optional_string(credential, "api_key", pointer, issues) {
                if !key.trim().is_empty() {
                    issues.push(issue(
                        &child(pointer, "api_key"),
                        "an environment credential must not carry a key",
                    ));
                }
            }
        }
        CredentialSource::None => {
            for field in ["api_key", "name"] {
                if let Some(text) = optional_string(credential, field, pointer, issues) {
                    if !text.trim().is_empty() {
                        issues.push(issue(
                            &child(pointer, field),
                            "a keyless credential must not carry a value",
                        ));
                    }
                }
            }
        }
    }
    Some(source)
}

/// Validates `rate_limits` and returns the `(quota_group_id, scope)` keys that
/// later reference checks join on.
fn validate_rate_limits(
    value: Option<&Value>,
    issues: &mut Vec<ConfigIssue>,
) -> Vec<(String, String)> {
    let mut keys: Vec<(String, String)> = Vec::new();
    let Some(items) = array_value(value, "/rate_limits", issues) else {
        return keys;
    };
    for (index, item) in items.iter().enumerate() {
        let pointer = entry("/rate_limits", index);
        let Some(obj) = object_value(Some(item), &pointer, issues) else {
            continue;
        };
        check_known_fields(obj, QUOTA_FIELDS, &pointer, issues);
        check_required(obj, &["quota_group_id"], &pointer, issues);
        let group = nonempty(
            required_string(obj, "quota_group_id", &pointer, issues),
            &child(&pointer, "quota_group_id"),
            issues,
        );
        let scope = optional_string(obj, "scope", &pointer, issues)
            .map(|scope| scope.trim().to_string())
            .filter(|scope| !scope.is_empty())
            .unwrap_or_else(|| "*".to_string());
        for key in ["verified_rpm", "verified_tpm", "verified_rpd"] {
            check_limit(obj, key, &pointer, false, None, issues);
        }
        // An explicit zero is the documented "disabled" quota.
        check_limit(obj, "local_rpm", &pointer, true, None, issues);
        check_limit(obj, "concurrency", &pointer, false, Some(64), issues);
        if let Some(source) = optional_string(obj, "source", &pointer, issues) {
            if !QUOTA_SOURCES.contains(&source) {
                issues.push(issue(
                    &child(&pointer, "source"),
                    format!("must be one of {}", QUOTA_SOURCES.join(", ")),
                ));
            }
        }
        if let Some(at) = optional_string(obj, "verified_at", &pointer, issues) {
            if !at.trim().is_empty() && chrono::DateTime::parse_from_rfc3339(at.trim()).is_err() {
                issues.push(issue(
                    &child(&pointer, "verified_at"),
                    "must be an RFC3339 timestamp",
                ));
            }
        }
        if let Some(group) = group {
            let key = (group.to_string(), scope);
            if keys.contains(&key) {
                issues.push(issue(&pointer, "duplicate quota group and scope"));
            } else {
                keys.push(key);
            }
        }
    }
    keys
}

/// Validates `providers` and returns the provider ids later reference checks
/// join on.
fn validate_providers(
    value: Option<&Value>,
    quota_keys: &[(String, String)],
    warnings: &mut Vec<ConfigIssue>,
    issues: &mut Vec<ConfigIssue>,
) -> Vec<String> {
    let mut ids: Vec<String> = Vec::new();
    let Some(items) = array_value(value, "/providers", issues) else {
        return ids;
    };
    for (index, item) in items.iter().enumerate() {
        let pointer = entry("/providers", index);
        let Some(obj) = object_value(Some(item), &pointer, issues) else {
            continue;
        };
        check_known_fields(obj, PROVIDER_FIELDS, &pointer, issues);
        check_required(
            obj,
            &["id", "kind", "base_url", "default_model", "credential"],
            &pointer,
            issues,
        );
        let id = nonempty(
            required_string(obj, "id", &pointer, issues),
            &child(&pointer, "id"),
            issues,
        );
        let kind = nonempty(
            required_string(obj, "kind", &pointer, issues),
            &child(&pointer, "kind"),
            issues,
        );
        let base_url = required_string(obj, "base_url", &pointer, issues);
        let source = object_value(
            obj.get("credential"),
            &child(&pointer, "credential"),
            issues,
        )
        .and_then(|credential| {
            validate_credential(credential, &child(&pointer, "credential"), warnings, issues)
        });
        if let Some(id) = id {
            if ids.iter().any(|seen| seen == id) {
                issues.push(issue(&child(&pointer, "id"), "duplicate provider id"));
            } else {
                ids.push(id.to_string());
            }
        }
        if let Some(kind) = kind {
            if !PROVIDER_KINDS.contains(&kind) {
                issues.push(issue(
                    &child(&pointer, "kind"),
                    format!("must be one of {}", PROVIDER_KINDS.join(", ")),
                ));
            }
            if let Some(source) = source {
                if kind == "subscription" && source != CredentialSource::None {
                    issues.push(issue(
                        &child(&pointer, "credential"),
                        "a subscription route never carries key material; use source \"none\"",
                    ));
                }
                if source == CredentialSource::None && kind != "subscription" && kind != "local" {
                    warnings.push(issue(
                        &child(&pointer, "credential"),
                        "keyless route: the provider needs a credential to be reachable",
                    ));
                }
            }
            if let Some(url) = base_url {
                check_scheme(url, kind, &child(&pointer, "base_url"), issues);
            }
        }
        if let Some(group) = optional_string(obj, "quota_group_id", &pointer, issues) {
            if !group.trim().is_empty()
                && !quota_keys.iter().any(|(known, _)| known == group.trim())
            {
                issues.push(issue(
                    &child(&pointer, "quota_group_id"),
                    "references a quota group that rate_limits does not define",
                ));
            }
        }
    }
    ids
}

fn validate_tool_credentials(
    value: Option<&Value>,
    warnings: &mut Vec<ConfigIssue>,
    issues: &mut Vec<ConfigIssue>,
) {
    let Some(items) = array_value(value, "/tool_credentials", issues) else {
        return;
    };
    let mut seen: Vec<String> = Vec::new();
    for (index, item) in items.iter().enumerate() {
        let pointer = entry("/tool_credentials", index);
        let Some(obj) = object_value(Some(item), &pointer, issues) else {
            continue;
        };
        check_known_fields(obj, TOOL_FIELDS, &pointer, issues);
        // `provider` is the join key, so it is the only required field: a slot the
        // document does not mention is not a supplied value, and must not be
        // overwritten. An absent slot therefore preserves the local account.
        check_required(obj, &["provider"], &pointer, issues);
        let provider = nonempty(
            required_string(obj, "provider", &pointer, issues),
            &child(&pointer, "provider"),
            issues,
        );
        if let Some(provider) = provider {
            if !TOOL_CREDENTIAL_PROVIDERS.contains(&provider) {
                issues.push(issue(
                    &child(&pointer, "provider"),
                    format!(
                        "must be one of the keyed tool providers: {}",
                        TOOL_CREDENTIAL_PROVIDERS.join(", ")
                    ),
                ));
            }
            if seen.iter().any(|known| known == provider) {
                issues.push(issue(
                    &child(&pointer, "provider"),
                    "duplicate tool credential entry",
                ));
            } else {
                seen.push(provider.to_string());
            }
        }
        for field in ["primary", "fallback"] {
            match obj.get(field) {
                None | Some(Value::Null) => {}
                Some(Value::Object(credential)) => {
                    validate_credential(credential, &child(&pointer, field), warnings, issues);
                }
                Some(_) => issues.push(issue(
                    &child(&pointer, field),
                    "must be a credential object or null",
                )),
            }
        }
    }
}

fn validate_route(
    route: &Map<String, Value>,
    pointer: &str,
    issues: &mut Vec<ConfigIssue>,
) -> Route {
    check_known_fields(route, ROUTE_FIELDS, pointer, issues);
    check_required(route, &["provider_id", "model"], pointer, issues);
    let provider_id = nonempty(
        required_string(route, "provider_id", pointer, issues),
        &child(pointer, "provider_id"),
        issues,
    )
    .unwrap_or_default()
    .to_string();
    let model = nonempty(
        required_string(route, "model", pointer, issues),
        &child(pointer, "model"),
        issues,
    )
    .unwrap_or_default()
    .to_string();
    Route { provider_id, model }
}

/// Validates one role assignment and returns its primary route, so the caller
/// can check role compatibility and reference integrity.
fn validate_role_assignment(
    assignment: &Map<String, Value>,
    pointer: &str,
    issues: &mut Vec<ConfigIssue>,
) -> Vec<Route> {
    check_known_fields(assignment, ROLE_ASSIGNMENT_FIELDS, pointer, issues);
    check_required(assignment, &["primary"], pointer, issues);
    let mut routes = Vec::new();
    match assignment.get("primary") {
        None | Some(Value::Null) => {}
        Some(Value::Object(route)) => {
            routes.push(validate_route(route, &child(pointer, "primary"), issues));
        }
        Some(_) => issues.push(issue(
            &child(pointer, "primary"),
            "must be a route object or null",
        )),
    }
    if let Some(items) = array_value(
        assignment.get("fallbacks"),
        &child(pointer, "fallbacks"),
        issues,
    ) {
        let mut labels: Vec<String> = Vec::new();
        for (index, item) in items.iter().enumerate() {
            let here = entry(&child(pointer, "fallbacks"), index);
            match item {
                Value::Object(route) => {
                    let route = validate_route(route, &here, issues);
                    let label = format!("{}::{}", route.provider_id, route.model);
                    if labels.contains(&label) {
                        issues.push(issue(&here, "duplicate fallback route"));
                    } else {
                        labels.push(label);
                    }
                }
                _ => issues.push(issue(&here, "must be a route object")),
            }
        }
    }
    routes
}

fn check_role_compatibility(
    role: &str,
    routes: &[Route],
    pointer: &str,
    issues: &mut Vec<ConfigIssue>,
) {
    for route in routes {
        let decisions = provider::is_decisions_model(&route.model);
        if DECISION_ROLES.contains(&role) && !decisions {
            issues.push(issue(
                &child(pointer, "primary"),
                format!(
                    "the {role} role needs a decisions model; {} is not one",
                    route.model
                ),
            ));
        }
        if GENERAL_ROLES.contains(&role) && decisions {
            issues.push(issue(
                &child(pointer, "primary"),
                format!("a decisions model cannot serve the {role} role"),
            ));
        }
    }
}

fn check_route_reference(
    route: &Route,
    provider_ids: &[String],
    pointer: &str,
    issues: &mut Vec<ConfigIssue>,
) {
    if route.provider_id.is_empty() {
        return;
    }
    let normalized = provider::normalize_kind(&route.provider_id);
    let known = provider_ids
        .iter()
        .any(|id| provider::normalize_kind(id) == normalized)
        || provider::preset(&normalized).is_some()
        || matches!(
            normalized.as_str(),
            "grok" | "grok-subscription" | "openai" | "openai-chatgpt" | "local"
        );
    if !known {
        issues.push(issue(
            &child(pointer, "primary"),
            format!("references an unknown provider: {}", route.provider_id),
        ));
    }
}

fn validate_roles(value: Option<&Value>, provider_ids: &[String], issues: &mut Vec<ConfigIssue>) {
    let Some(obj) = object_value(value, "/model_roles", issues) else {
        return;
    };
    check_known_fields(obj, ROLE_FIELDS, "/model_roles", issues);
    check_required(obj, ROLE_KEYS, "/model_roles", issues);
    for key in ROLE_KEYS.iter().copied() {
        let pointer = child("/model_roles", key);
        match obj.get(key) {
            None | Some(Value::Null) => {}
            Some(Value::Object(assignment)) => {
                let routes = validate_role_assignment(assignment, &pointer, issues);
                check_role_compatibility(key, &routes, &pointer, issues);
                for route in &routes {
                    check_route_reference(route, provider_ids, &pointer, issues);
                }
            }
            Some(_) => issues.push(issue(&pointer, "must be a role assignment object")),
        }
    }
    if let Some(value) = obj.get("decision_fallback") {
        let pointer = child("/model_roles", "decision_fallback");
        match value {
            Value::Null => {}
            Value::Object(assignment) => {
                let routes = validate_role_assignment(assignment, &pointer, issues);
                check_role_compatibility("decision_fallback", &routes, &pointer, issues);
                for route in &routes {
                    check_route_reference(route, provider_ids, &pointer, issues);
                }
            }
            _ => issues.push(issue(&pointer, "must be a role assignment object or null")),
        }
    }
}

/// Validates a parsed document. Returns the blocking issues; `warnings` receives
/// the non-blocking ones (an unresolved environment reference, for example).
fn validate_document(value: &Value, warnings: &mut Vec<ConfigIssue>) -> Vec<ConfigIssue> {
    let mut issues: Vec<ConfigIssue> = Vec::new();
    let Some(root) = value.as_object() else {
        issues.push(issue("", "the document root must be a JSON object"));
        return issues;
    };
    check_known_fields(root, ROOT_FIELDS, "", &mut issues);
    check_required(root, ROOT_REQUIRED, "", &mut issues);
    if let Some(version) = root.get("schema_version") {
        match version.as_u64() {
            Some(1) => {}
            Some(other) => issues.push(issue(
                "/schema_version",
                format!("expected {SCHEMA_VERSION}, found {other}"),
            )),
            None => issues.push(issue(
                "/schema_version",
                format!("must be the integer {SCHEMA_VERSION}"),
            )),
        }
    }
    if let Some(at) = optional_string(root, "exported_at", "", &mut issues) {
        if !at.trim().is_empty() && chrono::DateTime::parse_from_rfc3339(at.trim()).is_err() {
            issues.push(issue("/exported_at", "must be an RFC3339 timestamp"));
        }
    }
    let quota_keys = validate_rate_limits(root.get("rate_limits"), &mut issues);
    let provider_ids =
        validate_providers(root.get("providers"), &quota_keys, warnings, &mut issues);
    validate_tool_credentials(root.get("tool_credentials"), warnings, &mut issues);
    validate_roles(root.get("model_roles"), &provider_ids, &mut issues);
    issues
}

/// Validates an already typed document, so a hand-built document cannot skip the
/// checks the JSON walk performs. The role-compatibility and reference rules
/// stay in [`validate_document`]: an exported live configuration is a valid
/// document even when an operator deliberately pointed a role at an unusual
/// model, and import is where the contract is enforced.
fn validate_config(config: &ProfileConfig, warnings: &mut Vec<ConfigIssue>) -> Vec<ConfigIssue> {
    let mut issues = Vec::new();
    if config.schema_version != SCHEMA_VERSION {
        issues.push(issue(
            "/schema_version",
            format!("expected {SCHEMA_VERSION}, found {}", config.schema_version),
        ));
    }
    let mut ids: Vec<String> = Vec::new();
    for (index, provider) in config.providers.iter().enumerate() {
        let pointer = entry("/providers", index);
        if provider.id.trim().is_empty() {
            issues.push(issue(&child(&pointer, "id"), "must not be empty"));
        } else if ids.contains(&provider.id) {
            issues.push(issue(&child(&pointer, "id"), "duplicate provider id"));
        } else {
            ids.push(provider.id.clone());
        }
    }
    let mut tools: Vec<String> = Vec::new();
    for (index, tool) in config.tool_credentials.iter().enumerate() {
        let pointer = entry("/tool_credentials", index);
        if tool.provider.trim().is_empty() {
            issues.push(issue(&child(&pointer, "provider"), "must not be empty"));
        } else if tools.contains(&tool.provider) {
            issues.push(issue(
                &child(&pointer, "provider"),
                "duplicate tool credential entry",
            ));
        } else {
            tools.push(tool.provider.clone());
        }
    }
    let mut keys: Vec<(String, String)> = Vec::new();
    for setting in &config.rate_limits {
        let key = merge_key(setting);
        if keys.contains(&key) {
            issues.push(issue(
                "/rate_limits",
                format!("duplicate quota group and scope: {} / {}", key.0, key.1),
            ));
        } else {
            keys.push(key);
        }
    }
    for credential in env_credentials(&config.providers, &config.tool_credentials) {
        if env::var(&credential.name).is_err() {
            warnings.push(issue(
                &credential.pointer,
                format!(
                    "the environment variable {} is not set here: the route stays unusable until it resolves",
                    credential.name
                ),
            ));
        }
    }
    issues
}

/// Where a document names an environment variable.
struct EnvCredential {
    pointer: String,
    name: String,
}

fn env_credentials(
    providers: &[ProviderEntry],
    tools: &[ToolCredentialEntry],
) -> Vec<EnvCredential> {
    let mut out = Vec::new();
    for (index, provider) in providers.iter().enumerate() {
        if provider.credential.source == CredentialSource::Env
            && !provider.credential.name.is_empty()
        {
            out.push(EnvCredential {
                pointer: child(&entry("/providers", index), "credential/name"),
                name: provider.credential.name.clone(),
            });
        }
    }
    for (index, tool) in tools.iter().enumerate() {
        let mut slots: Vec<(&str, Option<&Credential>)> = Vec::new();
        if let Supplied::Value(credential) = &tool.primary {
            slots.push(("primary", Some(credential)));
        }
        if let Supplied::Value(credential) = &tool.fallback {
            slots.push(("fallback", Some(credential)));
        }
        for (field, slot) in slots {
            if let Some(credential) = slot {
                if credential.source == CredentialSource::Env && !credential.name.is_empty() {
                    out.push(EnvCredential {
                        pointer: child(
                            &entry("/tool_credentials", index),
                            &format!("{field}/name"),
                        ),
                        name: credential.name.clone(),
                    });
                }
            }
        }
    }
    out
}

/// Renders issues as one bounded, redacted error line set.
fn report(issues: &[ConfigIssue]) -> String {
    let mut out = String::new();
    for item in issues.iter().take(20) {
        if !out.is_empty() {
            out.push_str("; ");
        }
        out.push_str(&item.to_string());
    }
    if issues.len() > 20 {
        out.push_str(&format!("; …and {} more", issues.len() - 20));
    }
    out
}

// ---------------------------------------------------------------------------
// Export
// ---------------------------------------------------------------------------

/// The kind a provider entry carries. Two stored subscription kinds collapse
/// onto the portable `subscription` marker, which never carries key material.
fn export_kind(secret: &ProviderSecret) -> String {
    match provider::effective_kind(secret).as_str() {
        "grok-subscription" | "openai-chatgpt" => "subscription".to_string(),
        other => other.to_string(),
    }
}

fn provider_entry(id: &str, secret: &ProviderSecret, quotas: &QuotaSettingsFile) -> ProviderEntry {
    let kind = export_kind(secret);
    let credential = match secret.api_key.as_deref() {
        Some(key) if !key.trim().is_empty() => Credential::inline(key.trim()),
        _ => Credential::none(),
    };
    let base_url = if secret.base_url.trim().is_empty() {
        provider::preset(&kind)
            .map(|preset| preset.base_url.to_string())
            .unwrap_or_default()
    } else {
        secret.base_url.trim().to_string()
    };
    let quota_group_id = if quotas.group(id).is_empty() {
        String::new()
    } else {
        id.to_string()
    };
    ProviderEntry {
        id: id.to_string(),
        kind,
        base_url,
        default_model: secret.model.trim().to_string(),
        credential,
        quota_group_id,
    }
}

/// `inline` for a saved key, otherwise a reference to the documented
/// environment variable when one is set, otherwise absent. An environment secret
/// is never copied into a portable document.
fn tool_credential(saved: &str, env_name: Option<&str>) -> Option<Credential> {
    if !saved.trim().is_empty() {
        return Some(Credential::inline(saved.trim()));
    }
    if let Some(name) = env_name {
        if env::var(name).is_ok() {
            return Some(Credential::env(name));
        }
    }
    None
}

fn tool_credential_entries(settings: &SettingsFile) -> Vec<ToolCredentialEntry> {
    provider::KEY_ENV
        .iter()
        .map(|(id, env_name)| {
            let fallback_env = provider::KEY_FALLBACK_ENV
                .iter()
                .find(|(fallback_id, _)| fallback_id == id)
                .map(|(_, name)| *name);
            ToolCredentialEntry {
                provider: (*id).to_string(),
                primary: tool_credential(settings.saved_key(id), Some(*env_name))
                    .map(Supplied::Value)
                    .unwrap_or(Supplied::Value(Credential::none())),
                fallback: tool_credential(settings.saved_fallback_key(id), fallback_env)
                    .map(Supplied::Value)
                    .unwrap_or(Supplied::Value(Credential::none())),
            }
        })
        .collect()
}

fn role_assignment(assignment: &ModelAssignment) -> RoleAssignment {
    let inherited = assignment.provider.trim().is_empty() && assignment.model.trim().is_empty();
    let primary = if inherited {
        None
    } else {
        Some(Route {
            provider_id: provider::normalize_kind(&assignment.provider),
            model: assignment.model.trim().to_string(),
        })
    };
    let fallbacks = assignment
        .fallbacks
        .iter()
        .map(|route| Route {
            provider_id: provider::normalize_kind(&route.provider),
            model: route.model.trim().to_string(),
        })
        .collect();
    RoleAssignment {
        primary,
        fallbacks: Supplied::Value(fallbacks),
    }
}

/// Builds the portable document for the live configuration. Nothing ephemeral
/// travels: no counters, cooldowns, queues, databases or host paths.
pub fn export_document(
    settings: &SettingsFile,
    auth: &AuthFile,
    quotas: &QuotaSettingsFile,
) -> ProfileConfig {
    let mut providers: Vec<ProviderEntry> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for (id, secret) in &auth.accounts {
        if seen.insert(id.clone()) {
            providers.push(provider_entry(id, secret, quotas));
        }
    }
    // A legacy text/voice slot still names a configured account.
    for slot in [&auth.text, &auth.voice].into_iter().flatten() {
        let kind = provider::effective_kind(slot);
        if !kind.is_empty() && seen.insert(kind.clone()) {
            providers.push(provider_entry(&kind, slot, quotas));
        }
    }
    let rate_limits = quotas
        .settings
        .iter()
        .map(|setting| {
            let mut setting = setting.clone();
            if setting.scope.trim().is_empty() {
                setting.scope = "*".to_string();
            }
            setting
        })
        .collect();
    ProfileConfig {
        schema_version: SCHEMA_VERSION,
        exported_at: Some(chrono::Utc::now().to_rfc3339()),
        providers,
        tool_credentials: tool_credential_entries(settings),
        model_roles: RoleDocument {
            recon: role_assignment(&settings.defaults.recon),
            synthesis: role_assignment(&settings.defaults.synthesis),
            tool_picker: role_assignment(&settings.defaults.tool_picker),
            classifier: role_assignment(&settings.defaults.classifier),
            summarization: role_assignment(&settings.defaults.summarization),
            evidence_curator: role_assignment(&settings.defaults.evidence_curator),
            entity_resolver: role_assignment(&settings.defaults.entity_resolver),
            claim_assessor: role_assignment(&settings.defaults.claim_assessor),
            investigation_controller: role_assignment(&settings.defaults.investigation_controller),
            decision_model: role_assignment(&settings.defaults.decision_model),
            decision_fallback: settings
                .defaults
                .decision_fallback
                .as_ref()
                .map(role_assignment),
        },
        rate_limits,
    }
}

/// Serialises a document deterministically: field order is fixed and every
/// collection is built in a stable order, so the same configuration always
/// produces the same bytes.
pub fn serialize_document(document: &ProfileConfig) -> Result<String> {
    let mut warnings = Vec::new();
    let issues = validate_config(document, &mut warnings);
    if !issues.is_empty() {
        bail!(
            "the configuration document is not valid: {}",
            report(&issues)
        );
    }
    let mut body = serde_json::to_string_pretty(document)?;
    body.push('\n');
    Ok(body)
}

/// Writes a document to `path` through the secure primitive: a unique sibling
/// temporary file is created owner-only at creation, written, flushed, fsynced
/// and atomically renamed into place.
pub fn write_export(document: &ProfileConfig, path: &Path) -> Result<()> {
    let body = serialize_document(document)?;
    crate::config_commit::write_secure(path, &body)
}

// ---------------------------------------------------------------------------
// Import
// ---------------------------------------------------------------------------

/// Parses a document and returns the validated plan plus everything the Import
/// screen shows before the user commits.
///
/// The whole document is validated before anything is returned, so a caller can
/// never apply a half-valid document. Errors carry a JSON pointer and never
/// echo a value from the document.
pub fn parse_document(text: &str) -> Result<ImportPlan> {
    if text.len() > MAX_DOCUMENT_BYTES {
        bail!(
            "the document is larger than the {} byte import cap",
            MAX_DOCUMENT_BYTES
        );
    }
    let value = serde_json::from_str::<NoDuplicates>(text)
        .map(|parsed| parsed.0)
        .map_err(|err| anyhow::anyhow!("the document is not valid JSON: {err}"))?;
    let mut warnings: Vec<ConfigIssue> = Vec::new();
    let issues = validate_document(&value, &mut warnings);
    if !issues.is_empty() {
        bail!(
            "the configuration document is not valid: {}",
            report(&issues)
        );
    }
    let config: ProfileConfig = serde_json::from_value(value)
        .context("the configuration document does not match the schema")?;
    let mut typed_warnings = Vec::new();
    let typed_issues = validate_config(&config, &mut typed_warnings);
    if !typed_issues.is_empty() {
        bail!(
            "the configuration document is not valid: {}",
            report(&typed_issues)
        );
    }
    warnings.extend(typed_warnings);
    Ok(ImportPlan {
        config,
        warnings,
        changes: Vec::new(),
    })
}

/// The stored provider kind for an imported account. An existing account keeps
/// its stored kind, so the account identity (and its device endpoints) is
/// stable across imports.
fn stored_kind(document_kind: &str, base_url: &str, existing: Option<&ProviderSecret>) -> String {
    if let Some(existing) = existing {
        let kind = provider::normalize_kind(&existing.kind);
        if !kind.is_empty() && provider::effective_kind(existing) == kind {
            return existing.kind.clone();
        }
    }
    match document_kind {
        "subscription" => {
            let host = url::Url::parse(base_url.trim())
                .ok()
                .and_then(|url| url.host_str().map(|host| host.to_ascii_lowercase()))
                .unwrap_or_default();
            if host.contains("grok") || host.contains("x.ai") {
                "grok-subscription".to_string()
            } else {
                "openai-chatgpt".to_string()
            }
        }
        other => provider::normalize_kind(other),
    }
}

fn apply_provider(entry: &ProviderEntry, auth: &mut AuthFile) {
    let existing = auth.account(&entry.id);
    let kind = stored_kind(&entry.kind, &entry.base_url, existing.as_ref());
    let mut secret = existing.unwrap_or(ProviderSecret {
        kind: kind.clone(),
        base_url: String::new(),
        model: String::new(),
        api_key: None,
        stt_model: None,
        device: None,
    });
    secret.kind = kind;
    if !entry.base_url.trim().is_empty() {
        secret.base_url = entry.base_url.trim().to_string();
    }
    if !entry.default_model.trim().is_empty() {
        secret.model = entry.default_model.trim().to_string();
    }
    // An inline key is copied; an environment reference and a keyless route both
    // leave no saved key, because a saved key overrides the environment.
    secret.api_key = match entry.credential.source {
        CredentialSource::Inline => {
            Some(entry.credential.api_key.trim().to_string()).filter(|key| !key.is_empty())
        }
        CredentialSource::Env | CredentialSource::None => None,
    };
    auth.set_account(secret);
}

/// One tool credential slot. `Absent` preserves the local value; `Removed` and
/// an `env`/`none` credential all leave no saved key, because a saved key
/// overrides the environment.
/// One tool credential slot. `Absent` preserves the local value; `Removed` clears
/// it; an `inline` credential sets it; an `env` credential clears the local key
/// **only when the variable resolves here**, because an unresolved reference must
/// never land as an empty key — it keeps the local account and warns instead.
fn apply_tool_slot(
    slot: &Supplied<Credential>,
    settings: &mut SettingsFile,
    id: &str,
    fallback: bool,
) {
    let value = match slot {
        Supplied::Absent => return,
        Supplied::Removed => String::new(),
        Supplied::Value(Credential {
            source: CredentialSource::Inline,
            api_key,
            ..
        }) => api_key.trim().to_string(),
        // A keyless route means no account is configured.
        Supplied::Value(Credential {
            source: CredentialSource::None,
            ..
        }) => String::new(),
        // A resolvable environment reference takes over from the saved key.
        Supplied::Value(Credential {
            source: CredentialSource::Env,
            name,
            ..
        }) if std::env::var(name).is_ok() => String::new(),
        // An unresolved env reference keeps the local account: importing it
        // would silently install an empty key.
        Supplied::Value(_) => return,
    };
    if fallback {
        settings.set_saved_fallback_key(id, &value);
    } else {
        settings.set_saved_key(id, &value);
    }
}

fn apply_role(assignment: &RoleAssignment, target: Option<&mut ModelAssignment>) {
    let Some(target) = target else {
        return;
    };
    match &assignment.primary {
        Some(route) => {
            target.provider = provider::normalize_kind(&route.provider_id);
            target.model = route.model.trim().to_string();
            // The portable route carries no account label.
            target.account = String::new();
        }
        // An explicit null returns the role to documented inheritance.
        None => {
            target.provider = String::new();
            target.model = String::new();
            target.account = String::new();
            target.fallbacks = Vec::new();
            return;
        }
    }
    if let Supplied::Value(routes) = &assignment.fallbacks {
        target.fallbacks = routes
            .iter()
            .map(|route| ModelRoute {
                provider: provider::normalize_kind(&route.provider_id),
                model: route.model.trim().to_string(),
                account: String::new(),
            })
            .collect();
    }
}

/// Merges a validated plan into the live configuration. Nothing is written here:
/// the caller publishes the result with [`commit_profile_config`].
pub fn apply_import(
    plan: &ImportPlan,
    settings: &mut SettingsFile,
    auth: &mut AuthFile,
    quotas: &mut QuotaSettingsFile,
) -> Result<()> {
    let mut warnings = Vec::new();
    let issues = validate_config(&plan.config, &mut warnings);
    if !issues.is_empty() {
        bail!(
            "the configuration document is not valid: {}",
            report(&issues)
        );
    }
    for entry in &plan.config.providers {
        apply_provider(entry, auth);
    }
    for entry in &plan.config.tool_credentials {
        apply_tool_slot(&entry.primary, settings, &entry.provider, false);
        apply_tool_slot(&entry.fallback, settings, &entry.provider, true);
    }
    for (role, assignment) in plan.config.model_roles.entries() {
        apply_role(assignment, settings.defaults.role_mut(role));
    }
    match &plan.config.model_roles.decision_fallback {
        Some(assignment) => {
            if settings.defaults.decision_fallback.is_none() {
                settings.defaults.decision_fallback = Some(ModelAssignment::default());
            }
            apply_role(assignment, settings.defaults.decision_fallback.as_mut());
        }
        // An absent or null decision fallback means none configured.
        None => settings.defaults.decision_fallback = None,
    }
    for setting in &plan.config.rate_limits {
        quotas.upsert(setting.clone());
    }
    Ok(())
}

/// The three files one configuration revision is made of. A snapshot is what the
/// change summary compares before and after an import.
#[derive(Clone, Debug, Default)]
pub struct ConfigurationSnapshot {
    pub settings: SettingsFile,
    pub auth: AuthFile,
    pub quotas: QuotaSettingsFile,
}

impl ConfigurationSnapshot {
    /// Reads the live configuration. Never writes.
    pub fn load() -> Result<Self> {
        Ok(Self {
            settings: SettingsFile::load()?,
            auth: AuthFile::load()?,
            quotas: QuotaSettingsFile::load()?,
        })
    }

    /// Applies a validated plan in place.
    pub fn apply(&mut self, plan: &ImportPlan) -> Result<()> {
        apply_import(plan, &mut self.settings, &mut self.auth, &mut self.quotas)
    }

    /// Publishes this snapshot as one all-or-nothing revision.
    pub fn commit(&self) -> Result<u64> {
        commit_profile_config(&self.settings, &self.auth, &self.quotas)
    }
}

/// The redacted change summary: which areas, ids and fields move. No entry ever
/// carries a credential value.
fn describe_changes(
    document: &ProfileConfig,
    before: &ConfigurationSnapshot,
    after: &ConfigurationSnapshot,
) -> Vec<ConfigChange> {
    let mut changes: Vec<ConfigChange> = Vec::new();
    for entry in &document.providers {
        let before_secret = before.auth.account(&entry.id);
        let after_secret = after.auth.account(&entry.id);
        let same = match (&before_secret, &after_secret) {
            (None, None) => true,
            (Some(a), Some(b)) => {
                a.kind == b.kind
                    && a.base_url == b.base_url
                    && a.model == b.model
                    && a.api_key == b.api_key
            }
            _ => false,
        };
        if same {
            continue;
        }
        let credential = match entry.credential.source {
            CredentialSource::Inline => "key set".to_string(),
            CredentialSource::Env => {
                format!("environment variable {}", entry.credential.name)
            }
            CredentialSource::None => "no key".to_string(),
        };
        changes.push(ConfigChange {
            area: "provider".to_string(),
            id: entry.id.clone(),
            summary: format!(
                "{credential}; {} at {}",
                entry.default_model, entry.base_url
            ),
        });
    }
    for tool in &document.tool_credentials {
        for (field, slot) in [("primary", &tool.primary), ("fallback", &tool.fallback)] {
            if let Supplied::Absent = slot {
                continue;
            }
            let label = match slot {
                Supplied::Removed => format!("{field} cleared"),
                Supplied::Value(Credential {
                    source: CredentialSource::Inline,
                    ..
                }) => format!("{field} key set"),
                Supplied::Value(credential) => {
                    format!("{field} {}", credential.source.as_str())
                }
                Supplied::Absent => unreachable!(),
            };
            changes.push(ConfigChange {
                area: "tool".to_string(),
                id: tool.provider.clone(),
                summary: label,
            });
        }
    }
    for (role, assignment) in document.model_roles.entries() {
        let before_role = before
            .settings
            .defaults
            .role(role)
            .cloned()
            .unwrap_or_default();
        let after_role = after
            .settings
            .defaults
            .role(role)
            .cloned()
            .unwrap_or_default();
        if before_role == after_role {
            continue;
        }
        let summary = match &assignment.primary {
            None => "returns to inherited default".to_string(),
            Some(route) => format!("{} · {}", route.provider_id, route.model),
        };
        changes.push(ConfigChange {
            area: "role".to_string(),
            id: role.to_string(),
            summary,
        });
    }
    if before.settings.defaults.decision_fallback != after.settings.defaults.decision_fallback {
        let summary = match document
            .model_roles
            .decision_fallback
            .as_ref()
            .and_then(|a| a.primary.as_ref())
        {
            Some(route) => format!("{} · {}", route.provider_id, route.model),
            None => "cleared".to_string(),
        };
        changes.push(ConfigChange {
            area: "role".to_string(),
            id: "decision_fallback".to_string(),
            summary,
        });
    }
    for setting in &after.quotas.settings {
        let key = merge_key(setting);
        let existing = before
            .quotas
            .settings
            .iter()
            .find(|before_setting| merge_key(before_setting) == key);
        let (verb, changed) = match existing {
            None => ("quota group added", true),
            Some(before_setting) => ("quota group updated", before_setting != setting),
        };
        if changed {
            changes.push(ConfigChange {
                area: "quota".to_string(),
                id: format!("{} / {}", key.0, key.1),
                summary: verb.to_string(),
            });
        }
    }
    changes
}

/// Publishes settings, auth and quota settings as one all-or-nothing
/// configuration revision under the app-wide write lock.
pub fn commit_profile_config(
    settings: &SettingsFile,
    auth: &AuthFile,
    quotas: &QuotaSettingsFile,
) -> Result<u64> {
    paths::ensure_home()?;
    let lock = ConfigLock::acquire()?;
    lock.commit_files(&[
        CommitFile::new(
            "settings",
            paths::config_path(),
            toml::to_string_pretty(settings)?,
        ),
        CommitFile::new(
            "auth",
            paths::auth_path(),
            serde_json::to_string_pretty(auth)?,
        ),
        CommitFile::new(
            "quota",
            paths::quota_path(),
            serde_json::to_string_pretty(quotas)?,
        ),
    ])
}

/// Exports the live configuration, writes it to `path`, and returns the document.
pub fn export_to_path(
    settings: &SettingsFile,
    auth: &AuthFile,
    quotas: &QuotaSettingsFile,
    path: &Path,
) -> Result<ProfileConfig> {
    let document = export_document(settings, auth, quotas);
    write_export(&document, path)?;
    Ok(document)
}

/// Reads a document from `path` and returns the validated import plan.
pub fn import_from_path(
    path: &Path,
    settings: &SettingsFile,
    auth: &AuthFile,
    quotas: &QuotaSettingsFile,
) -> Result<ImportPlan> {
    let text = std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let plan = parse_document(&text)?;
    let changes = plan.changes_against(settings, auth, quotas);
    Ok(ImportPlan {
        config: plan.config,
        warnings: plan.warnings,
        changes,
    })
}
