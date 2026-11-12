//! Additive gateway discovery for the native cutover.
//!
//! A decoded field is capability metadata, not permission or service liveness.
//! Older payloads can decode so the client can show an upgrade explanation;
//! missing required contracts must never activate a compatibility bypass.

use std::fmt;

use serde::Deserialize;
use serde::Serialize;
use url::Url;

use crate::connect::CONNECT_CONTRACT_VERSION;
use crate::context::Permission;
use crate::context::ResourceScope;
use crate::search::ACTOR_SEARCH_CONTRACT_VERSION;

/// Additive registry feature advertised with the current Connect contract.
pub const CONNECT_REGISTRY_FEATURE: &str = "connect-registry-v1";

/// Existing and additive capabilities from the configured gateway.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct Capabilities {
    pub api_version: u16,
    #[serde(default)]
    pub categories: Vec<String>,
    #[serde(default)]
    pub auth: bool,
    #[serde(default)]
    pub features: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub live: Option<LiveCapabilities>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instance_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deployment_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connect: Option<ConnectCapabilities>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_control: Option<WorkspaceControlCapabilities>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_scopes: Option<ResourceScopes>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search: Option<SearchCapabilities>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub configuration_epoch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build: Option<BuildIdentity>,
    #[serde(default)]
    pub links: CapabilityLinks,
    /// Effective server grants for presentation; the server rechecks operations.
    #[serde(default)]
    pub permissions: Vec<Permission>,
}

impl Default for Capabilities {
    fn default() -> Self {
        Self {
            api_version: crate::API_VERSION,
            categories: Vec::new(),
            auth: false,
            features: Vec::new(),
            live: None,
            instance_id: None,
            deployment_id: None,
            connect: None,
            workspace_control: None,
            resource_scopes: None,
            search: None,
            configuration_epoch: None,
            build: None,
            links: CapabilityLinks::default(),
            permissions: Vec::new(),
        }
    }
}

impl Capabilities {
    /// Require the current registration, global/resource and reset contracts.
    ///
    /// This verifies advertised versions only. It does not authorize a caller,
    /// prove readiness, or verify that a deployment ID is stable across restart.
    pub fn require_native_contract(&self) -> Result<(), CapabilityError> {
        if self.api_version != crate::API_VERSION {
            return Err(CapabilityError::UnsupportedApiVersion(self.api_version));
        }
        if self.deployment_id.as_ref().is_none_or(String::is_empty) {
            return Err(CapabilityError::MissingDeploymentIdentity);
        }
        if self
            .configuration_epoch
            .as_ref()
            .is_none_or(String::is_empty)
        {
            return Err(CapabilityError::MissingConfigurationEpoch);
        }
        let connect = self
            .connect
            .as_ref()
            .ok_or(CapabilityError::MissingRegistry)?;
        if connect.version != CONNECT_CONTRACT_VERSION || !connect.registration_required {
            return Err(CapabilityError::UnsupportedRegistryVersion(connect.version));
        }
        if !self
            .features
            .iter()
            .any(|feature| feature == CONNECT_REGISTRY_FEATURE)
        {
            return Err(CapabilityError::MissingRegistry);
        }
        let workspace = self
            .workspace_control
            .as_ref()
            .ok_or(CapabilityError::MissingWorkspaceControl)?;
        if workspace.version != CONNECT_CONTRACT_VERSION
            || workspace.purpose != "registration_and_activity"
        {
            return Err(CapabilityError::UnsupportedWorkspaceControl);
        }
        if self.resource_scopes.as_ref() != Some(&ResourceScopes::default()) {
            return Err(CapabilityError::InvalidResourceScopes);
        }
        Ok(())
    }

    /// Require versioned Tantivy discovery independently of UI registration.
    pub fn require_actor_search(&self) -> Result<(), CapabilityError> {
        let actors = self
            .search
            .as_ref()
            .and_then(|search| search.actors.as_ref())
            .ok_or(CapabilityError::MissingActorSearch)?;
        if actors.version != ACTOR_SEARCH_CONTRACT_VERSION || actors.engine != "tantivy" {
            return Err(CapabilityError::UnsupportedActorSearch);
        }
        Ok(())
    }

    /// Whether presentation metadata includes a permission; never server authority.
    #[must_use]
    pub fn advertises_permission(&self, permission: Permission) -> bool {
        self.permissions.contains(&permission)
    }
}

/// Failure to advertise a supported current contract.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum CapabilityError {
    UnsupportedApiVersion(u16),
    MissingDeploymentIdentity,
    MissingConfigurationEpoch,
    MissingRegistry,
    UnsupportedRegistryVersion(u16),
    MissingWorkspaceControl,
    UnsupportedWorkspaceControl,
    InvalidResourceScopes,
    MissingActorSearch,
    UnsupportedActorSearch,
    InvalidWebLink,
}

impl fmt::Display for CapabilityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedApiVersion(version) => {
                write!(formatter, "unsupported gateway API version {version}")
            }
            Self::MissingDeploymentIdentity => {
                formatter.write_str("gateway does not advertise a stable deployment identity")
            }
            Self::MissingConfigurationEpoch => {
                formatter.write_str("gateway requires the current fresh-configuration contract")
            }
            Self::MissingRegistry => {
                formatter.write_str("gateway does not support registered Connect context")
            }
            Self::UnsupportedRegistryVersion(version) => {
                write!(formatter, "unsupported Connect registry version {version}")
            }
            Self::MissingWorkspaceControl => formatter
                .write_str("gateway does not advertise workspace registration/activity control"),
            Self::UnsupportedWorkspaceControl => {
                formatter.write_str("gateway workspace control contract is incompatible")
            }
            Self::InvalidResourceScopes => formatter
                .write_str("gateway resource and workstation activity scopes are incompatible"),
            Self::MissingActorSearch => {
                formatter.write_str("gateway does not advertise actor discovery search")
            }
            Self::UnsupportedActorSearch => {
                formatter.write_str("gateway actor discovery search contract is incompatible")
            }
            Self::InvalidWebLink => {
                formatter.write_str("gateway advertised an invalid or unsafe Intel Web destination")
            }
        }
    }
}

impl std::error::Error for CapabilityError {}

/// Existing live capability fields, retained alongside the cutover additions.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct LiveCapabilities {
    pub path: String,
    pub channels: Vec<String>,
    pub version: u16,
}

impl LiveCapabilities {
    /// Advertise the gateway's current live transport contract.
    #[must_use]
    pub fn new(path: String, channels: Vec<String>, version: u16) -> Self {
        Self {
            path,
            channels,
            version,
        }
    }
}

/// Current registration contract. It does not imply a caller has registration rights.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct ConnectCapabilities {
    pub version: u16,
    pub registration_required: bool,
}

impl Default for ConnectCapabilities {
    fn default() -> Self {
        Self {
            version: CONNECT_CONTRACT_VERSION,
            registration_required: true,
        }
    }
}

/// Workspace registry/activity capability; workspace configuration copies are absent.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct WorkspaceControlCapabilities {
    pub version: u16,
    pub purpose: String,
}

impl Default for WorkspaceControlCapabilities {
    fn default() -> Self {
        Self {
            version: CONNECT_CONTRACT_VERSION,
            purpose: "registration_and_activity".into(),
        }
    }
}

/// Required ownership metadata for the five global pages and native activity.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct ResourceScopes {
    pub atlas: ResourceScope,
    pub brain: ResourceScope,
    pub tools: ResourceScope,
    pub models: ResourceScope,
    pub profile: ResourceScope,
    pub native_jobs_logs: ResourceScope,
}

impl Default for ResourceScopes {
    fn default() -> Self {
        Self {
            atlas: ResourceScope::Backend,
            brain: ResourceScope::Backend,
            tools: ResourceScope::Backend,
            models: ResourceScope::Backend,
            profile: ResourceScope::Backend,
            native_jobs_logs: ResourceScope::WorkstationTriggeredActivity,
        }
    }
}

/// Search capabilities, kept independent from search readiness.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct SearchCapabilities {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actors: Option<ActorSearchCapabilities>,
}

/// Version and engine of gateway actor discovery, never a private index origin.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct ActorSearchCapabilities {
    pub version: u16,
    pub engine: String,
}

impl Default for ActorSearchCapabilities {
    fn default() -> Self {
        Self {
            version: ACTOR_SEARCH_CONTRACT_VERSION,
            engine: "tantivy".into(),
        }
    }
}

/// Public build identity; not the deployment's persisted identity.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct BuildIdentity {
    pub version: String,
    pub revision: String,
}

impl BuildIdentity {
    /// Describe exact source/version without claiming runtime verification.
    #[must_use]
    pub fn new(version: String, revision: String) -> Self {
        Self { version, revision }
    }
}

/// Optional external browser links; absence must not imply localhost fallback.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct CapabilityLinks {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intel_web: Option<IntelWebLinks>,
}

/// Advertised external web application and its explicit Connect form.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct IntelWebLinks {
    pub url: String,
    pub connect_url: String,
}

impl IntelWebLinks {
    /// Construct untrusted discovery metadata to validate before browser use.
    #[must_use]
    pub fn new(url: String, connect_url: String) -> Self {
        Self { url, connect_url }
    }

    /// Validate an advertised destination against local/remote transport policy.
    ///
    /// A changed web origin still requires the native confirmation policy.
    /// Returning a URL proves neither registration nor browser/service health.
    pub fn validated_connect_url(&self, gateway_origin: &str) -> Result<Url, CapabilityError> {
        let gateway = Url::parse(gateway_origin).map_err(|_| CapabilityError::InvalidWebLink)?;
        let web = Url::parse(&self.url).map_err(|_| CapabilityError::InvalidWebLink)?;
        let connect = Url::parse(&self.connect_url).map_err(|_| CapabilityError::InvalidWebLink)?;
        for url in [&web, &connect] {
            if url.host().is_none()
                || !matches!(url.scheme(), "http" | "https")
                || !url.username().is_empty()
                || url.password().is_some()
                || url.query().is_some()
                || url.fragment().is_some()
                || (url.scheme() == "http" && !(loopback(&gateway) && loopback(url)))
                || (!loopback(&gateway) && loopback(url))
            {
                return Err(CapabilityError::InvalidWebLink);
            }
        }
        if web.origin() != connect.origin() {
            return Err(CapabilityError::InvalidWebLink);
        }
        let base_path = web.path().trim_end_matches('/');
        if connect.path() != format!("{base_path}/connect") {
            return Err(CapabilityError::InvalidWebLink);
        }
        Ok(connect)
    }
}

fn loopback(url: &Url) -> bool {
    match url.host() {
        Some(url::Host::Domain(name)) => name == "localhost",
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn current() -> Capabilities {
        Capabilities {
            deployment_id: Some("deployment-a".into()),
            configuration_epoch: Some("fresh-config-v1".into()),
            features: vec![CONNECT_REGISTRY_FEATURE.into()],
            connect: Some(ConnectCapabilities::default()),
            workspace_control: Some(WorkspaceControlCapabilities::default()),
            resource_scopes: Some(ResourceScopes::default()),
            ..Capabilities::default()
        }
    }

    #[test]
    fn old_capabilities_decode_but_never_bypass_required_contracts() {
        let old: Capabilities = serde_json::from_value(serde_json::json!({"api_version":1,"categories":["news"],"auth":true,"features":["jobs"],"live":{"path":"/api/v1/live","channels":["metrics","brief"],"version":1}})).expect("existing response decodes");
        assert!(old.auth);
        assert_eq!(old.categories, ["news"]);
        assert_eq!(
            old.require_native_contract(),
            Err(CapabilityError::MissingDeploymentIdentity)
        );
        assert!(current().require_native_contract().is_ok());
    }

    #[test]
    fn scope_and_search_versions_are_not_inferred_from_feature_names() {
        let mut capabilities = current();
        capabilities
            .resource_scopes
            .as_mut()
            .expect("scopes")
            .models = ResourceScope::WorkstationTriggeredActivity;
        assert_eq!(
            capabilities.require_native_contract(),
            Err(CapabilityError::InvalidResourceScopes)
        );
        assert_eq!(
            capabilities.require_actor_search(),
            Err(CapabilityError::MissingActorSearch)
        );
        capabilities.search = Some(SearchCapabilities {
            actors: Some(ActorSearchCapabilities::default()),
        });
        assert!(capabilities.require_actor_search().is_ok());
        capabilities
            .search
            .as_mut()
            .expect("search")
            .actors
            .as_mut()
            .expect("actors")
            .engine = "direct-db-fallback".into();
        assert_eq!(
            capabilities.require_actor_search(),
            Err(CapabilityError::UnsupportedActorSearch)
        );
    }

    #[test]
    fn web_destination_preserves_base_path_and_remote_boundary() {
        let remote = IntelWebLinks::new(
            "https://intel.example.test/review/".into(),
            "https://intel.example.test/review/connect".into(),
        );
        assert!(remote
            .validated_connect_url("https://gateway.example.test")
            .is_ok());
        let local = IntelWebLinks::new(
            "http://127.0.0.1:3001/".into(),
            "http://127.0.0.1:3001/connect".into(),
        );
        assert!(local.validated_connect_url("http://127.0.0.1:8080").is_ok());
        assert!(local
            .validated_connect_url("https://gateway.example.test")
            .is_err());
        for unsafe_connect in [
            "https://elsewhere.example.test/review/connect",
            "https://intel.example.test/connect",
            "https://token@intel.example.test/review/connect",
            "https://intel.example.test/review/connect?token=secret",
            "javascript:alert(1)",
        ] {
            let links = IntelWebLinks::new(remote.url.clone(), unsafe_connect.into());
            assert!(links
                .validated_connect_url("https://gateway.example.test")
                .is_err());
        }
    }
}
