use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::Path;

use anyhow::{anyhow, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use toml::{Table, Value};

use crate::{optional_text, persist_new, Workspace, CONFIG_LIMIT};

const KEY_FIELDS: &[&str] = &[
    "firecrawl_api_key",
    "firecrawl_api_key_fallback",
    "hunter_api_key",
    "hunter_api_key_fallback",
    "sociavault_api_key",
    "sociavault_api_key_fallback",
    "newsapi_api_key",
    "newsapi_api_key_fallback",
    "courtlistener_api_token",
    "courtlistener_api_token_fallback",
    "gnews_api_key",
    "gnews_api_key_fallback",
    "newsdata_api_key",
    "newsdata_api_key_fallback",
    "currents_api_key",
    "currents_api_key_fallback",
    "whoxy_api_key",
    "whoxy_api_key_fallback",
];
const MODEL_FIELDS: &[&str] = &["model", "writer_model", "writer_provider"];
const ROLE_FIELDS: &[&str] = &[
    "recon_agent",
    "recon_decisions",
    "recon_analyst",
    "atlas_evaluator",
    "atlas_analyst",
    "brain_summary",
    "recon",
    "synthesis",
    "tool_picker",
    "classifier",
    "summarization",
    "evidence_curator",
    "entity_resolver",
    "claim_assessor",
    "decision_model",
    "decision_fallback",
];

/// Counts of imported entries. No key material or configuration values are returned.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct ImportSummary {
    pub api_keys: usize,
    pub model_defaults: usize,
}

#[derive(Default, Deserialize, Serialize)]
struct Accounts {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    text: Option<Account>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    voice: Option<Account>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    accounts: BTreeMap<String, Account>,
}

#[derive(Deserialize, Serialize)]
struct Account {
    kind: String,
    base_url: String,
    model: String,
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    stt_model: Option<String>,
}

impl Account {
    fn has_key(&self) -> bool {
        self.api_key
            .as_ref()
            .is_some_and(|key| !key.trim().is_empty())
    }
}

#[derive(Default, Deserialize, Serialize)]
#[serde(default)]
struct ModelRoute {
    provider: String,
    model: String,
    account: String,
}

#[derive(Default, Deserialize, Serialize)]
#[serde(default)]
struct ModelAssignment {
    #[serde(flatten)]
    route: ModelRoute,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    fallbacks: Vec<ModelRoute>,
}

#[derive(Deserialize, Serialize)]
#[serde(untagged)]
enum OperationOverride {
    Role { route_role: String },
    Assignment(ModelAssignment),
}

impl Workspace {
    /// Import allowed credentials and model routes from a legacy Argos home.
    /// Both destinations are checked before publishing. A repeat with the same
    /// input is safe, including after interruption between the two files.
    pub fn import_legacy(&self, source: &Path) -> Result<ImportSummary> {
        let source = fs::canonicalize(source).context("resolve legacy Argos home")?;
        ensure!(source.is_dir(), "legacy Argos home must be a directory");
        ensure!(
            !self.root().starts_with(&source),
            "workspace must be outside the legacy Argos home"
        );

        let config: Table = crate::read_toml(&source.join("config.toml"), false)?;
        let auth = optional_text(&source.join("auth.json"), CONFIG_LIMIT, false)?;
        let mut auth: Accounts = auth
            .map(|body| {
                serde_json::from_str(&body).map_err(|_| anyhow!("invalid legacy auth.json"))
            })
            .transpose()?
            .unwrap_or_default();
        auth.text = auth.text.filter(Account::has_key);
        auth.voice = auth.voice.filter(Account::has_key);
        auth.accounts.retain(|_, account| account.has_key());
        let auth_count = usize::from(auth.text.is_some())
            + usize::from(auth.voice.is_some())
            + auth.accounts.len();

        let (config, config_keys, model_defaults) = project_settings(&config)?;
        let mut outputs = Vec::new();
        if auth_count > 0 {
            outputs.push((
                self.root().join("auth.json"),
                serde_json::to_string_pretty(&auth).map_err(|_| anyhow!("encode API accounts"))?,
            ));
        }
        if !config.is_empty() {
            outputs.push((
                self.root().join("config.toml"),
                toml::to_string_pretty(&config).map_err(|_| anyhow!("encode legacy settings"))?,
            ));
        }
        for (path, body) in &outputs {
            check_destination(path, body)?;
        }
        self.initialize()?;
        for (path, body) in outputs {
            if check_destination(&path, &body)? {
                continue;
            }
            match persist_new(&path, body.as_bytes()) {
                Ok(()) => (),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    ensure!(
                        check_destination(&path, &body)?,
                        "legacy import destination changed during import"
                    );
                }
                Err(error) => return Err(error).context("write imported Argos configuration"),
            }
        }
        Ok(ImportSummary {
            api_keys: auth_count + config_keys,
            model_defaults,
        })
    }
}

fn check_destination(path: &Path, expected: &str) -> Result<bool> {
    let Some(existing) = optional_text(path, CONFIG_LIMIT, true)? else {
        return Ok(false);
    };
    ensure!(
        existing == expected,
        "workspace already has different {}",
        path.file_name()
            .expect("import destinations have a file name")
            .to_string_lossy()
    );
    Ok(true)
}

fn project_settings(source: &Table) -> Result<(Table, usize, usize)> {
    let mut output = Table::new();
    let mut keys = 0;
    let mut defaults = 0;
    for &name in KEY_FIELDS.iter().chain(MODEL_FIELDS) {
        let Some(value) = source.get(name) else {
            continue;
        };
        let value = value.as_str().context("invalid legacy setting")?;
        if !value.trim().is_empty() {
            output.insert(name.to_owned(), Value::String(value.to_owned()));
            if KEY_FIELDS.contains(&name) {
                keys += 1;
            } else {
                defaults += 1;
            }
        }
    }

    let Some(roles) = source.get("defaults") else {
        return Ok((output, keys, defaults));
    };
    let roles = roles.as_table().context("invalid legacy model defaults")?;
    let mut projected = Table::new();
    for &name in ROLE_FIELDS {
        if let Some(value) = roles.get(name) {
            let assignment: ModelAssignment = decode_setting(value)?;
            projected.insert(name.to_owned(), encode_setting(assignment)?);
            defaults += 1;
        }
    }
    if let Some(value) = roles.get("operation_overrides") {
        let overrides: BTreeMap<String, OperationOverride> = decode_setting(value)?;
        defaults += overrides.len();
        projected.insert("operation_overrides".to_owned(), encode_setting(overrides)?);
    }
    if let Some(value) = roles.get("model_variants") {
        let variants: BTreeMap<String, String> = decode_setting(value)?;
        defaults += variants.len();
        projected.insert("model_variants".to_owned(), encode_setting(variants)?);
    }
    if let Some(value) = roles.get("text_contract_version") {
        let version: String = decode_setting(value)?;
        projected.insert("text_contract_version".to_owned(), Value::String(version));
    }
    if !projected.is_empty() {
        output.insert("defaults".to_owned(), Value::Table(projected));
    }
    Ok((output, keys, defaults))
}

fn decode_setting<T: serde::de::DeserializeOwned>(value: &Value) -> Result<T> {
    value
        .clone()
        .try_into()
        .map_err(|_| anyhow!("invalid legacy model route"))
}

fn encode_setting(value: impl Serialize) -> Result<Value> {
    Value::try_from(value).map_err(|_| anyhow!("encode legacy model route"))
}
