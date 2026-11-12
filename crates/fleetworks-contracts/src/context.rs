//! Resource scope, purpose permissions and immutable interactive request origin.
//!
//! These wire types are not an authorization mechanism. Public callers cannot
//! assign ownership or system identity by submitting them. The gateway resolves
//! authenticated context, checks current revisions and persists origin in the
//! same durable admission boundary as its job or requester association.

use std::fmt;
use std::net::IpAddr;
use std::str::FromStr;

use serde::Deserialize;
use serde::Serialize;

use crate::connect::ConnectId;
use crate::connect::RegistrationReceipt;
use crate::connect::RegistrationStatus;
use crate::connect::WorkspaceName;

/// Version of server-enforced workstation-triggered activity semantics.
pub const ACTIVITY_SCOPE_VERSION: u16 = 1;

/// A purpose-scoped operation permission, always granted by server policy.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Permission {
    ReadBackend,
    RegisterWorkstation,
    RebindWorkstation,
    ResolveWebSession,
    ManageConfiguration,
    MaintainBackend,
    SubmitInteractive,
    ReadWorkstationActivity,
    RetryJob,
    CancelJob,
}

impl Permission {
    /// Stable permission spelling shared by credentials, DTOs and diagnostics.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ReadBackend => "read_backend",
            Self::RegisterWorkstation => "register_workstation",
            Self::RebindWorkstation => "rebind_workstation",
            Self::ResolveWebSession => "resolve_web_session",
            Self::ManageConfiguration => "manage_configuration",
            Self::MaintainBackend => "maintain_backend",
            Self::SubmitInteractive => "submit_interactive",
            Self::ReadWorkstationActivity => "read_workstation_activity",
            Self::RetryJob => "retry_job",
            Self::CancelJob => "cancel_job",
        }
    }
}

impl fmt::Display for Permission {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// An unsupported permission spelling. Its input is intentionally not echoed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UnknownPermission;

impl fmt::Display for UnknownPermission {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("unsupported permission")
    }
}

impl std::error::Error for UnknownPermission {}

impl FromStr for Permission {
    type Err = UnknownPermission;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "read_backend" => Ok(Self::ReadBackend),
            "register_workstation" => Ok(Self::RegisterWorkstation),
            "rebind_workstation" => Ok(Self::RebindWorkstation),
            "resolve_web_session" => Ok(Self::ResolveWebSession),
            "manage_configuration" => Ok(Self::ManageConfiguration),
            "maintain_backend" => Ok(Self::MaintainBackend),
            "submit_interactive" => Ok(Self::SubmitInteractive),
            "read_workstation_activity" => Ok(Self::ReadWorkstationActivity),
            "retry_job" => Ok(Self::RetryJob),
            "cancel_job" => Ok(Self::CancelJob),
            _ => Err(UnknownPermission),
        }
    }
}

/// Credential audience/purpose, independent of its effective permissions.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum CredentialPurpose {
    Native,
    WebRelay,
    InternalService,
    Operator,
}

/// Scope of the affected resource; it does not describe the requester.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ResourceScope {
    Backend,
    WorkstationTriggeredActivity,
}

impl ResourceScope {
    /// Stable scope spelling used in capabilities and operation receipts.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Backend => "backend",
            Self::WorkstationTriggeredActivity => "workstation_triggered_activity",
        }
    }
}

/// Non-secret context resolved by the gateway from current authority.
///
/// Registration, session context, activity scope and configuration revisions are
/// deliberately separate. This value is a snapshot; server boundaries must
/// revalidate it before admission, reads, subscriptions and protected actions.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct RegisteredContext {
    pub deployment_id: String,
    pub registration_id: String,
    pub workstation_id: String,
    pub workstation_ip: IpAddr,
    pub workspace_id: String,
    pub workspace_name: WorkspaceName,
    pub connect_id: ConnectId,
    pub registration_revision: u64,
    pub context_revision: u64,
    pub scope_revision: u64,
    pub configuration_epoch: String,
    pub access_epoch: String,
}

/// Invalid construction of a context snapshot, with no sensitive input echoed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ContextError {
    InvalidRegistration,
    InactiveRegistration,
    InvalidRevision,
    MissingEpoch,
}

impl fmt::Display for ContextError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidRegistration => "registration identity or snapshot is inconsistent",
            Self::InactiveRegistration => "registration is not active",
            Self::InvalidRevision => "context revisions must be positive",
            Self::MissingEpoch => "current configuration and access epochs are required",
        })
    }
}

impl std::error::Error for ContextError {}

impl RegisteredContext {
    /// Construct a snapshot from a server-resolved active registration.
    pub fn new(
        registration: RegistrationReceipt,
        context_revision: u64,
        scope_revision: u64,
        configuration_epoch: String,
        access_epoch: String,
    ) -> Result<Self, ContextError> {
        if !registration.is_consistent() {
            return Err(ContextError::InvalidRegistration);
        }
        if registration.status != RegistrationStatus::Active {
            return Err(ContextError::InactiveRegistration);
        }
        if context_revision == 0 || scope_revision == 0 {
            return Err(ContextError::InvalidRevision);
        }
        if configuration_epoch.is_empty() || access_epoch.is_empty() {
            return Err(ContextError::MissingEpoch);
        }
        Ok(Self {
            deployment_id: registration.deployment_id,
            registration_id: registration.registration_id,
            workstation_id: registration.workstation_id,
            workstation_ip: registration.workstation_ip,
            workspace_id: registration.workspace_id,
            workspace_name: registration.workspace_name,
            connect_id: registration.connect_id,
            registration_revision: registration.revision,
            context_revision,
            scope_revision,
            configuration_epoch,
            access_epoch,
        })
    }

    /// Whether a response still belongs to the same registered client context.
    ///
    /// This is only a client-side stale-reply fence, not an authorization check.
    #[must_use]
    pub fn same_request_context(&self, other: &Self) -> bool {
        self.deployment_id == other.deployment_id
            && self.registration_id == other.registration_id
            && self.registration_revision == other.registration_revision
            && self.context_revision == other.context_revision
            && self.scope_revision == other.scope_revision
            && self.access_epoch == other.access_epoch
            && self.configuration_epoch == other.configuration_epoch
    }
}

/// Registered context/revision expected by a mutation, never caller-assigned origin.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct MutationContext {
    pub registration_id: String,
    pub expected_registration_revision: u64,
    pub expected_context_revision: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_configuration_revision: Option<u64>,
}

impl MutationContext {
    /// Pin a new request to the context the user confirmed.
    #[must_use]
    pub fn new(context: &RegisteredContext) -> Self {
        Self {
            registration_id: context.registration_id.clone(),
            expected_registration_revision: context.registration_revision,
            expected_context_revision: context.context_revision,
            expected_configuration_revision: None,
        }
    }

    /// Add the independently versioned global configuration precondition.
    #[must_use]
    pub fn with_configuration_revision(mut self, revision: u64) -> Self {
        self.expected_configuration_revision = Some(revision);
        self
    }
}

/// Only interactive sources are constructible through this origin contract.
///
/// Trusted scheduler/system admission uses an explicit private service policy;
/// it must not invent a registration or allow a public `source=system` bypass.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum OriginSource {
    Native,
    IntelWeb,
}

/// Immutable non-secret origin captured together with durable acceptance.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct JobOrigin {
    pub deployment_id: String,
    pub registration_id: String,
    pub workstation_id: String,
    pub workspace_id: String,
    pub connect_id_at_submission: ConnectId,
    pub workstation_ip_at_submission: IpAddr,
    pub workspace_name_at_submission: WorkspaceName,
    pub source: OriginSource,
    pub request_id: String,
    pub submitted_at: String,
    pub configuration_epoch: String,
    pub configuration_revision: u64,
    /// Opaque credential references only; never API keys or bearer bytes.
    pub credential_references: Vec<String>,
}

impl JobOrigin {
    /// Capture a server-validated interactive context and effective configuration.
    #[must_use]
    pub fn new(
        context: &RegisteredContext,
        source: OriginSource,
        request_id: String,
        submitted_at: String,
        configuration_revision: u64,
        credential_references: Vec<String>,
    ) -> Self {
        Self {
            deployment_id: context.deployment_id.clone(),
            registration_id: context.registration_id.clone(),
            workstation_id: context.workstation_id.clone(),
            workspace_id: context.workspace_id.clone(),
            connect_id_at_submission: context.connect_id.clone(),
            workstation_ip_at_submission: context.workstation_ip,
            workspace_name_at_submission: context.workspace_name.clone(),
            source,
            request_id,
            submitted_at,
            configuration_epoch: context.configuration_epoch.clone(),
            configuration_revision,
            credential_references,
        }
    }
}

/// A genuine request associated with a reused job, without rewriting its creator.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct JobRequestAssociation {
    pub job_id: String,
    pub origin: JobOrigin,
    pub associated_at: String,
}

impl JobRequestAssociation {
    /// Record a request-to-job association at the durable admission boundary.
    #[must_use]
    pub fn new(job_id: String, origin: JobOrigin, associated_at: String) -> Self {
        Self {
            job_id,
            origin,
            associated_at,
        }
    }
}

/// Required workstation activity scope with an optional narrowing child filter.
///
/// This type has no all-backend/all-node/system mode. The server resolves and
/// authorizes the node and every child filter before querying or streaming.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ActivityScope {
    pub deployment_id: String,
    pub workstation_id: String,
    pub scope_revision: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
}

impl ActivityScope {
    /// Show the selected workstation's eligible activity across its children.
    #[must_use]
    pub fn for_workstation(context: &RegisteredContext) -> Self {
        Self {
            deployment_id: context.deployment_id.clone(),
            workstation_id: context.workstation_id.clone(),
            scope_revision: context.scope_revision,
            workspace_id: None,
        }
    }

    /// Narrow the same node scope to the currently selected child.
    #[must_use]
    pub fn for_selected_workspace(context: &RegisteredContext) -> Self {
        Self {
            workspace_id: Some(context.workspace_id.clone()),
            ..Self::for_workstation(context)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context(child: &str, registration: &str) -> RegisteredContext {
        let id = format!("192.0.2.42:{child}")
            .parse()
            .expect("synthetic Connect ID");
        let receipt = RegistrationReceipt::new(
            "deployment-a".into(),
            "node-a".into(),
            child.into(),
            registration.into(),
            id,
            3,
        );
        RegisteredContext::new(receipt, 7, 11, "reset-1".into(), "access-2".into())
            .expect("synthetic context")
    }

    #[test]
    fn permission_spellings_are_explicit_and_unknown_values_fail_closed() {
        for permission in [
            Permission::ReadBackend,
            Permission::RegisterWorkstation,
            Permission::RebindWorkstation,
            Permission::ResolveWebSession,
            Permission::ManageConfiguration,
            Permission::MaintainBackend,
            Permission::SubmitInteractive,
            Permission::ReadWorkstationActivity,
            Permission::RetryJob,
            Permission::CancelJob,
        ] {
            assert_eq!(permission.as_str().parse::<Permission>(), Ok(permission));
            assert_eq!(
                serde_json::to_value(permission).expect("serialize permission"),
                permission.as_str()
            );
        }
        for unknown in ["", "admin", "*", "native", "ReadBackend"] {
            assert!(unknown.parse::<Permission>().is_err());
            assert!(serde_json::from_value::<Permission>(serde_json::json!(unknown)).is_err());
        }
    }

    #[test]
    fn immutable_job_origin_survives_later_workspace_and_ip_switching() {
        let mut current = context("research", "registration-a");
        let origin = JobOrigin::new(
            &current,
            OriginSource::Native,
            "request-1".into(),
            "2026-01-01T00:00:00Z".into(),
            19,
            vec!["account-ref".into()],
        );
        current.workspace_name = WorkspaceName::new("review").expect("child");
        current.workstation_ip = "192.0.2.99".parse().expect("IP");
        current.registration_id = "registration-b".into();
        assert_eq!(origin.workspace_name_at_submission.as_str(), "research");
        assert_eq!(
            origin.workstation_ip_at_submission.to_string(),
            "192.0.2.42"
        );
        assert_eq!(origin.registration_id, "registration-a");
        assert_eq!(origin.configuration_revision, 19);
    }

    #[test]
    fn activity_child_filter_only_narrows_and_replies_fence_on_context_revision() {
        let first = context("research", "registration-a");
        let second = context("review", "registration-b");
        let all = ActivityScope::for_workstation(&first);
        let child = ActivityScope::for_selected_workspace(&first);
        assert_eq!(all.workstation_id, child.workstation_id);
        assert_eq!(all.deployment_id, child.deployment_id);
        assert_eq!(all.scope_revision, child.scope_revision);
        assert!(all.workspace_id.is_none());
        assert_eq!(child.workspace_id.as_deref(), Some("research"));
        assert!(first.same_request_context(&first));
        assert!(!first.same_request_context(&second));
        let mut stale = first.clone();
        stale.context_revision += 1;
        assert!(!first.same_request_context(&stale));
        assert!(
            serde_json::from_value::<ActivityScope>(serde_json::json!({"all_nodes":true})).is_err()
        );
    }

    #[test]
    fn mutation_input_cannot_assign_source_or_immutable_origin() {
        let valid =
            serde_json::to_value(MutationContext::new(&context("research", "registration-a")))
                .expect("context input");
        for field in ["source", "origin", "owner", "workstation_id"] {
            let mut forged = valid.clone();
            forged[field] = serde_json::json!("forged");
            assert!(
                serde_json::from_value::<MutationContext>(forged).is_err(),
                "field {field}"
            );
        }
        assert!(serde_json::from_value::<OriginSource>(serde_json::json!("system")).is_err());
    }
}
