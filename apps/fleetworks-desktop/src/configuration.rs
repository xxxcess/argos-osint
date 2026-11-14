//! Confirmed global configuration writes with optimistic backend revisions.

use std::collections::BTreeMap;

use fleetworks_contracts::context::Permission;
use fleetworks_contracts::Client;
use fleetworks_contracts::ClientError;
use rune::TextEdit;
use serde::Deserialize;
use serde::Serialize;

use crate::app::Intent;
use crate::app::Load;
use crate::app::State;
use crate::backend::ModelRoute;
use crate::backend::ReadRequest;
use crate::connection::GatewayEffects;
use crate::connection::RequestTask;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConfigurationAction {
    ModelRole,
    ProviderAccount,
    DeleteAccount,
    ToolCredentials,
}

#[derive(Serialize)]
pub struct SetModelRole {
    pub expected_revision: u64,
    pub role: String,
    pub primary: Option<ModelRoute>,
    pub fallbacks: Vec<ModelRoute>,
    pub variant_updates: BTreeMap<String, Option<String>>,
}

#[derive(Serialize)]
pub struct UpsertAccount {
    pub expected_revision: u64,
    pub account_id: String,
    pub provider: String,
    pub base_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
}

#[derive(Serialize)]
pub struct DeleteAccount {
    pub expected_revision: u64,
    pub account_id: String,
}

#[derive(Serialize)]
pub struct SetToolCredentials {
    pub expected_revision: u64,
    pub provider: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub primary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fallback: Option<String>,
}

/// Secret-bearing changes deliberately have no Debug or fixture intent payload.
pub enum ConfigurationChange {
    ModelRole(SetModelRole),
    Account(UpsertAccount),
    DeleteAccount(DeleteAccount),
    ToolCredentials(SetToolCredentials),
}

impl ConfigurationChange {
    pub fn action(&self) -> ConfigurationAction {
        match self {
            Self::ModelRole(_) => ConfigurationAction::ModelRole,
            Self::Account(_) => ConfigurationAction::ProviderAccount,
            Self::DeleteAccount(_) => ConfigurationAction::DeleteAccount,
            Self::ToolCredentials(_) => ConfigurationAction::ToolCredentials,
        }
    }
    pub fn expected_revision(&self) -> u64 {
        match self {
            Self::ModelRole(request) => request.expected_revision,
            Self::Account(request) => request.expected_revision,
            Self::DeleteAccount(request) => request.expected_revision,
            Self::ToolCredentials(request) => request.expected_revision,
        }
    }
    pub fn summary(&self) -> String {
        match self {
            Self::ModelRole(request) => format!(
                "Role {} · primary {} · {} ordered fallbacks",
                request.role,
                request
                    .primary
                    .as_ref()
                    .map_or("clear assignment".into(), ModelRoute::label),
                request.fallbacks.len()
            ),
            Self::Account(request) => format!(
                "Account {} · {} · {} · key {}",
                request.account_id,
                request.provider,
                request.base_url,
                match &request.api_key {
                    None => "preserved only if provider and base URL are unchanged",
                    Some(key) if key.is_empty() => "explicitly cleared",
                    Some(_) => "replaced with the entered value",
                }
            ),
            Self::DeleteAccount(request) => format!(
                "Delete account {}. The backend rejects accounts still referenced by model roles.",
                request.account_id
            ),
            Self::ToolCredentials(request) => format!(
                "{} tool credentials · primary {} · fallback {}",
                request.provider,
                match &request.primary {
                    None => "preserved",
                    Some(value) if value.is_empty() => "cleared",
                    Some(_) => "replaced",
                },
                match &request.fallback {
                    None => "preserved",
                    Some(value) if value.is_empty() => "cleared",
                    Some(_) => "replaced",
                }
            ),
        }
    }
}

pub struct ConfigurationConfirmation {
    pub change: ConfigurationChange,
    pub origin_revision: u64,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ConfigurationSaved {
    pub saved: bool,
    pub configuration_revision: u64,
}

pub struct Configuration {
    pub role: TextEdit,
    pub model_provider: TextEdit,
    pub model: TextEdit,
    pub model_account: TextEdit,
    pub model_extra: BTreeMap<String, serde_json::Value>,
    pub model_picker: Option<String>,
    pub variant_updates: BTreeMap<String, Option<String>>,
    pub fallbacks: TextEdit,
    pub account_id: TextEdit,
    pub account_provider: TextEdit,
    pub account_base_url: TextEdit,
    pub account_model: TextEdit,
    pub account_api_key: TextEdit,
    pub preserve_account_key: bool,
    pub tool_provider: TextEdit,
    pub tool_primary: TextEdit,
    pub tool_fallback: TextEdit,
    pub clear_tool_primary: bool,
    pub clear_tool_fallback: bool,
    pub confirmation: Option<ConfigurationConfirmation>,
    pub result: Load<ConfigurationSaved>,
    pub message: Option<String>,
    pub(crate) request: Option<RequestTask>,
    pub(crate) revision: u64,
    pub(crate) draft_revision: u64,
    pub(crate) saved_draft_revision: u64,
}

fn secret() -> TextEdit {
    let mut edit = TextEdit::new("");
    edit.set_secure(true);
    edit
}

impl Default for Configuration {
    fn default() -> Self {
        Self {
            role: TextEdit::new(""),
            model_provider: TextEdit::new(""),
            model: TextEdit::new(""),
            model_account: TextEdit::new(""),
            model_extra: BTreeMap::new(),
            model_picker: None,
            variant_updates: BTreeMap::new(),
            fallbacks: TextEdit::new("[]"),
            account_id: TextEdit::new(""),
            account_provider: TextEdit::new(""),
            account_base_url: TextEdit::new(""),
            account_model: TextEdit::new(""),
            account_api_key: secret(),
            preserve_account_key: true,
            tool_provider: TextEdit::new(""),
            tool_primary: secret(),
            tool_fallback: secret(),
            clear_tool_primary: false,
            clear_tool_fallback: false,
            confirmation: None,
            result: Load::Idle,
            message: None,
            request: None,
            revision: 0,
            draft_revision: 0,
            saved_draft_revision: 0,
        }
    }
}

impl State {
    pub fn configuration_edited(&mut self) {
        self.configuration.confirmation = None;
        self.configuration.draft_revision = self.configuration.draft_revision.wrapping_add(1);
    }

    pub fn edit_model_role(&mut self, role: &str) {
        let Load::Ready(defaults) = &self.backend.models else {
            return;
        };
        if !defaults.roles.iter().any(|candidate| candidate == role) {
            return;
        }
        let assignment = defaults.assignment(role).unwrap_or_default();
        self.configuration.role = TextEdit::new(role);
        self.configuration.model_provider = TextEdit::new(assignment.primary.provider);
        self.configuration.model = TextEdit::new(assignment.primary.model);
        self.configuration.model_account = TextEdit::new(assignment.primary.account);
        self.configuration.model_extra = assignment.primary.extra;
        self.configuration.variant_updates.clear();
        self.configuration.model_picker = None;
        self.configuration.fallbacks = TextEdit::new(
            serde_json::to_string_pretty(&assignment.fallbacks).unwrap_or_else(|_| "[]".into()),
        );
        self.configuration_edited();
    }

    pub fn select_model_provider(&mut self, provider: &str) {
        let Load::Ready(defaults) = &self.backend.models else {
            return;
        };
        let matches = defaults
            .catalog
            .iter()
            .filter(|entry| entry.provider == provider)
            .collect::<Vec<_>>();
        let Some(selected) = matches.first() else {
            return;
        };
        self.configuration.model_provider = TextEdit::new(provider);
        if matches.len() == 1 {
            self.configuration.model_account = TextEdit::new(&selected.account);
            self.configuration.model = TextEdit::new(&selected.model);
        } else {
            self.configuration.model_account = TextEdit::new("");
            self.configuration.model = TextEdit::new("");
        }
        self.configuration.model_extra.clear();
        self.configuration.model_picker = None;
        self.configuration_edited();
    }

    pub fn select_catalog_model(&mut self, label: &str) {
        let Load::Ready(defaults) = &self.backend.models else {
            return;
        };
        let Some(selected) = defaults.catalog.iter().find(|entry| {
            entry.provider == self.configuration.model_provider.text && entry.label() == label
        }) else {
            return;
        };
        self.configuration.model = TextEdit::new(&selected.model);
        self.configuration.model_account = TextEdit::new(&selected.account);
        self.configuration.model_extra.clear();
        self.configuration.model_picker = None;
        self.configuration_edited();
    }

    pub fn select_model_variant(&mut self, value: &str) {
        let Load::Ready(defaults) = &self.backend.models else {
            return;
        };
        let Some(entry) = defaults.catalog.iter().find(|entry| {
            entry.provider == self.configuration.model_provider.text
                && entry.account == self.configuration.model_account.text
                && entry.model == self.configuration.model.text
        }) else {
            return;
        };
        if value != "Provider default" && !entry.efforts.iter().any(|effort| effort == value) {
            return;
        }
        let key = format!(
            "{}|{}|{}",
            self.configuration.role.text, entry.provider, entry.model
        );
        self.configuration
            .variant_updates
            .insert(key, (value != "Provider default").then(|| value.to_owned()));
        self.configuration.model_picker = None;
        self.configuration_edited();
    }

    pub fn add_model_fallback(&mut self, label: &str) {
        let Load::Ready(defaults) = &self.backend.models else {
            return;
        };
        let Some(entry) = defaults
            .catalog
            .iter()
            .find(|entry| format!("{} · {}", entry.provider, entry.label()) == label)
        else {
            return;
        };
        let Ok(mut routes) =
            serde_json::from_str::<Vec<ModelRoute>>(&self.configuration.fallbacks.text)
        else {
            return;
        };
        if routes.len() >= 16 {
            return;
        }
        routes.push(ModelRoute {
            provider: entry.provider.clone(),
            model: entry.model.clone(),
            account: entry.account.clone(),
            extra: BTreeMap::new(),
        });
        self.configuration.fallbacks =
            TextEdit::new(serde_json::to_string(&routes).unwrap_or_else(|_| "[]".into()));
        self.configuration.model_picker = None;
        self.configuration_edited();
    }

    pub fn reorder_model_fallback(&mut self, index: usize, delta: isize) {
        let Ok(mut routes) =
            serde_json::from_str::<Vec<ModelRoute>>(&self.configuration.fallbacks.text)
        else {
            return;
        };
        if index >= routes.len() {
            return;
        }
        if delta == 0 {
            routes.remove(index);
        } else if let Some(destination) = index
            .checked_add_signed(delta)
            .filter(|destination| *destination < routes.len())
        {
            routes.swap(index, destination);
        }
        self.configuration.fallbacks =
            TextEdit::new(serde_json::to_string(&routes).unwrap_or_else(|_| "[]".into()));
        self.configuration_edited();
    }

    pub fn preview_model_role(&mut self, clear: bool) {
        let Load::Ready(defaults) = &self.backend.models else {
            self.configuration.message =
                Some("Refresh model roles before changing an assignment".into());
            return;
        };
        let role = self.configuration.role.text.trim();
        if !defaults.roles.iter().any(|candidate| candidate == role) {
            self.configuration.message = Some("Choose a canonical role from this backend".into());
            return;
        }
        let mut primary = (!clear).then(|| ModelRoute {
            provider: self.configuration.model_provider.text.trim().to_owned(),
            model: self.configuration.model.text.trim().to_owned(),
            account: self.configuration.model_account.text.trim().to_owned(),
            extra: BTreeMap::new(),
        });
        let _ = &primary;
        let mut fallbacks: Vec<ModelRoute> = if clear {
            Vec::new()
        } else {
            match serde_json::from_str(&self.configuration.fallbacks.text) {
                Ok(routes) => routes,
                Err(_) => {
                    self.configuration.message = Some(
                        "Enter an ordered JSON array of fallback provider/model/account routes"
                            .into(),
                    );
                    return;
                }
            }
        };
        for route in &mut fallbacks {
            route.extra.clear();
        }
        if let Some(route) = &mut primary {
            route.extra.clear();
        }
        if fallbacks.len() > 16
            || primary.iter().chain(fallbacks.iter()).any(|route| {
                route.provider.is_empty()
                    || route.model.is_empty()
                    || route.provider.len() > 128
                    || route.model.len() > 256
                    || route.account.len() > 128
            })
        {
            self.configuration.message = Some(
                "Each route needs a provider and model, with at most sixteen ordered fallbacks"
                    .into(),
            );
            return;
        }
        let active_variant_keys = primary
            .iter()
            .chain(fallbacks.iter())
            .map(|route| format!("{role}|{}|{}", route.provider, route.model))
            .collect::<std::collections::BTreeSet<_>>();
        // Pending edits for a replaced route are no longer part of this save.
        // Existing server variants, including other roles, remain untouched.
        self.configuration
            .variant_updates
            .retain(|key, _| active_variant_keys.contains(key));
        self.preview_configuration(ConfigurationChange::ModelRole(SetModelRole {
            expected_revision: defaults.configuration_revision,
            role: role.to_owned(),
            primary,
            fallbacks,
            variant_updates: self.configuration.variant_updates.clone(),
        }));
    }

    pub fn edit_provider_account(&mut self, account_id: &str) {
        let Load::Ready(profile) = &self.backend.profile else {
            return;
        };
        let Some(account) = profile
            .accounts
            .iter()
            .find(|account| account.id == account_id)
        else {
            return;
        };
        self.configuration.account_id = TextEdit::new(&account.id);
        self.configuration.account_provider = TextEdit::new(&account.provider);
        self.configuration.account_base_url = TextEdit::new(&account.base_url);
        self.configuration.account_model = TextEdit::new(&account.model);
        self.configuration.account_api_key.replace_text("");
        self.configuration.preserve_account_key = true;
        self.configuration_edited();
    }

    pub fn preview_provider_account(&mut self) {
        let Load::Ready(profile) = &self.backend.profile else {
            self.configuration.message =
                Some("Refresh backend profile before changing an account".into());
            return;
        };
        let editor = &self.configuration;
        let account_id = editor.account_id.text.trim();
        if account_id.is_empty()
            || editor.account_provider.text.trim().is_empty()
            || editor.account_base_url.text.trim().is_empty()
        {
            self.configuration.message = Some("Enter the account ID, provider and base URL".into());
            return;
        }
        let valid_url = url::Url::parse(editor.account_base_url.text.trim()).is_ok_and(|url| {
            let loopback_http = url.scheme() == "http"
                && url.host_str().is_some_and(|host| {
                    host.eq_ignore_ascii_case("localhost")
                        || host
                            .trim_matches(['[', ']'])
                            .parse::<std::net::IpAddr>()
                            .is_ok_and(|ip| ip.is_loopback())
                });
            (url.scheme() == "https" || loopback_http)
                && url.host().is_some()
                && url.username().is_empty()
                && url.password().is_none()
                && url.query().is_none()
                && url.fragment().is_none()
        });
        if !valid_url {
            self.configuration.message = Some("Enter an HTTPS provider base URL (or HTTP for localhost/loopback) without embedded credentials, query or fragment".into());
            return;
        }
        let request = UpsertAccount {
            expected_revision: profile.configuration_revision,
            account_id: account_id.to_owned(),
            provider: editor.account_provider.text.trim().to_owned(),
            base_url: editor.account_base_url.text.trim().to_owned(),
            model: (!editor.account_model.text.trim().is_empty())
                .then(|| editor.account_model.text.trim().to_owned()),
            api_key: if editor.account_api_key.text.is_empty() && editor.preserve_account_key {
                None
            } else {
                Some(editor.account_api_key.text.clone())
            },
        };
        self.preview_configuration(ConfigurationChange::Account(request));
    }

    pub fn preview_delete_account(&mut self, account_id: &str) {
        let Load::Ready(profile) = &self.backend.profile else {
            return;
        };
        if !profile
            .accounts
            .iter()
            .any(|account| account.id == account_id)
        {
            return;
        }
        self.preview_configuration(ConfigurationChange::DeleteAccount(DeleteAccount {
            expected_revision: profile.configuration_revision,
            account_id: account_id.to_owned(),
        }));
    }

    pub fn preview_tool_credentials(&mut self) {
        let Load::Ready(profile) = &self.backend.profile else {
            return;
        };
        let provider = self.configuration.tool_provider.text.trim();
        if !profile
            .tool_credentials
            .iter()
            .any(|credential| credential.provider == provider)
        {
            self.configuration.message =
                Some("Choose a tool credential provider advertised by this backend".into());
            return;
        }
        self.preview_configuration(ConfigurationChange::ToolCredentials(SetToolCredentials {
            expected_revision: profile.configuration_revision,
            provider: provider.to_owned(),
            primary: if self.configuration.clear_tool_primary {
                Some(String::new())
            } else {
                (!self.configuration.tool_primary.text.is_empty())
                    .then(|| self.configuration.tool_primary.text.clone())
            },
            fallback: if self.configuration.clear_tool_fallback {
                Some(String::new())
            } else {
                (!self.configuration.tool_fallback.text.is_empty())
                    .then(|| self.configuration.tool_fallback.text.clone())
            },
        }));
    }

    fn preview_configuration(&mut self, change: ConfigurationChange) {
        if !self.permission(Permission::ManageConfiguration) {
            self.configuration.message = Some("Backend configuration permission required".into());
            return;
        }
        if matches!(self.configuration.result, Load::Loading) {
            return;
        }
        if self.configuration_revision() != Some(change.expected_revision()) {
            self.configuration.message =
                Some("Refresh configuration metadata before reviewing this change".into());
            return;
        }
        self.configuration.confirmation = Some(ConfigurationConfirmation {
            change,
            origin_revision: self.origin_revision,
        });
        self.reveal_control("configuration-confirmation");
        self.configuration.message = None;
    }

    pub fn confirm_configuration(&mut self) {
        let Some(confirmation) = self.configuration.confirmation.take() else {
            return;
        };
        if !self.permission(Permission::ManageConfiguration)
            || confirmation.origin_revision != self.origin_revision
            || self.configuration_revision() != Some(confirmation.change.expected_revision())
        {
            self.configuration.message = Some(
                "Context or backend revision changed; refresh and review this change again".into(),
            );
            return;
        }
        self.record(Intent::Configuration {
            action: confirmation.change.action(),
        });
        if self.fixture {
            return;
        }
        let (Some(services), Some(client)) = (&self.services, &self.client) else {
            self.configuration.result =
                Load::Failed("Gateway configuration services are unavailable".into());
            return;
        };
        self.configuration.revision = self.configuration.revision.wrapping_add(1);
        self.configuration.result = Load::Loading;
        self.configuration.request = services.configuration(
            client.clone(),
            confirmation.change,
            self.connection.epoch,
            self.configuration.revision,
            self.configuration.draft_revision,
        );
    }
}

impl GatewayEffects {
    fn configuration(
        &self,
        client: Client,
        change: ConfigurationChange,
        epoch: u64,
        revision: u64,
        draft_revision: u64,
    ) -> Option<RequestTask> {
        let ui = self.ui.get()?.clone();
        let task = self.runtime.spawn(async move {
            let expected_revision = change.expected_revision();
            let action = change.action();
            let result: Result<ConfigurationSaved, ClientError> = match change {
                ConfigurationChange::ModelRole(request) => client.post("/api/v1/models/defaults/set", &request).await,
                ConfigurationChange::Account(request) => client.post("/api/v1/profile/accounts/upsert", &request).await,
                ConfigurationChange::DeleteAccount(request) => client.post("/api/v1/profile/accounts/delete", &request).await,
                ConfigurationChange::ToolCredentials(request) => client.post("/api/v1/profile/tools/set", &request).await,
            };
            ui.update(move |state, _| {
                if state.connection.epoch != epoch || state.configuration.revision != revision { return; }
                state.configuration.request = None;
                let conflict = matches!(&result, Err(ClientError::Status(409)));
                state.configuration.result = match result {
                    Ok(saved) if saved.saved && saved.configuration_revision > expected_revision => {
                        if state.configuration.draft_revision == draft_revision {
                            state.configuration.saved_draft_revision = draft_revision;
                            match action {
                                ConfigurationAction::ProviderAccount => state.configuration.account_api_key.replace_text(""),
                                ConfigurationAction::ToolCredentials => { state.configuration.tool_primary.replace_text(""); state.configuration.tool_fallback.replace_text(""); },
                                _ => {},
                            }
                        }
                        state.maintenance.confirmation = None;
                        state.submissions.invalidate_confirmations();
                        state.classifier_activation.invalidate_confirmation();
                        Load::Ready(saved)
                    }
                    Ok(_) => Load::Failed("Gateway returned an invalid configuration receipt".into()),
                    Err(ClientError::Status(409)) => Load::Failed("Backend configuration changed or needs an explicit current-format reset; refresh and review before retrying".into()),
                    Err(error) => Load::Failed(error.to_string()),
                };
                if conflict || matches!(state.configuration.result, Load::Ready(_)) {
                    state.backend_read(ReadRequest::Profile); state.backend_read(ReadRequest::Models);
                }
            });
        });
        Some(RequestTask(task.abort_handle()))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::SetModelRole;
    use crate::backend::ModelRoute;

    #[test]
    fn set_model_role_omits_extra_fields_on_routes_for_strict_server_deserializer() {
        let request = SetModelRole {
            expected_revision: 2,
            role: "extractor".into(),
            primary: Some(ModelRoute {
                provider: "openai".into(),
                account: "acct-1".into(),
                model: "gpt-4o".into(),
                extra: BTreeMap::new(),
            }),
            fallbacks: vec![ModelRoute {
                provider: "openai".into(),
                account: "acct-2".into(),
                model: "gpt-4o-mini".into(),
                extra: BTreeMap::new(),
            }],
            variant_updates: BTreeMap::new(),
        };
        let value = serde_json::to_value(request).expect("serialize model role");
        let primary = value["primary"].as_object().expect("primary object");
        assert_eq!(primary.len(), 3);
        assert!(primary.contains_key("provider"));
        assert!(primary.contains_key("account"));
        assert!(primary.contains_key("model"));
        let fallback = value["fallbacks"][0].as_object().expect("fallback object");
        assert_eq!(fallback.len(), 3);
        assert!(fallback.contains_key("provider"));
        assert!(fallback.contains_key("account"));
        assert!(fallback.contains_key("model"));
    }
}
