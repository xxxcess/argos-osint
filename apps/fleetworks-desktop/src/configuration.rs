//! Confirmed global configuration writes with optimistic backend revisions.

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
    pub primary: String,
    pub fallback: String,
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
                if request.primary.is_empty() {
                    "cleared"
                } else {
                    "replaced"
                },
                if request.fallback.is_empty() {
                    "cleared"
                } else {
                    "replaced"
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
    pub confirmation: Option<ConfigurationConfirmation>,
    pub result: Load<ConfigurationSaved>,
    pub message: Option<String>,
    pub(crate) request: Option<RequestTask>,
    pub(crate) revision: u64,
    pub(crate) draft_revision: u64,
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
            confirmation: None,
            result: Load::Idle,
            message: None,
            request: None,
            revision: 0,
            draft_revision: 0,
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
        self.configuration.fallbacks = TextEdit::new(
            serde_json::to_string_pretty(&assignment.fallbacks).unwrap_or_else(|_| "[]".into()),
        );
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
        let primary = (!clear).then(|| ModelRoute {
            provider: self.configuration.model_provider.text.trim().to_owned(),
            model: self.configuration.model.text.trim().to_owned(),
            account: self.configuration.model_account.text.trim().to_owned(),
        });
        let fallbacks: Vec<ModelRoute> = if clear {
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
        self.preview_configuration(ConfigurationChange::ModelRole(SetModelRole {
            expected_revision: defaults.configuration_revision,
            role: role.to_owned(),
            primary,
            fallbacks,
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
            matches!(url.scheme(), "http" | "https")
                && url.host().is_some()
                && url.username().is_empty()
                && url.password().is_none()
                && url.query().is_none()
                && url.fragment().is_none()
        });
        if !valid_url {
            self.configuration.message = Some("Enter an HTTP(S) provider base URL without embedded credentials, query or fragment".into());
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
            primary: self.configuration.tool_primary.text.clone(),
            fallback: self.configuration.tool_fallback.text.clone(),
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
