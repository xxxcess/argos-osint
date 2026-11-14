//! Server-captured interactive provenance, never accepted as a public DTO.

use std::ops::Deref;

use argos_osint_core::config_commit::ConfigLock;
use argos_osint_core::provider::SettingsFile;
use argos_osint_core::storage::job_context::InteractiveAdmission;
use chrono::Utc;
use fleetworks_contracts::context::JobOrigin;

use crate::configuration;
use crate::request_context::AdmissionProof;
use crate::request_context::ResolvedRequest;
use crate::ApiError;

/// Keep the captured snapshot serialized with every configuration writer until
/// the caller's durable acceptance transaction has completed. Clones retain the
/// same file guard; no transport DTO can construct or serialize this owner.
#[derive(Clone)]
pub(crate) struct CapturedAdmission {
    admission: InteractiveAdmission,
    _configuration_lock: ConfigLock,
}

impl Deref for CapturedAdmission {
    type Target = InteractiveAdmission;

    fn deref(&self) -> &Self::Target {
        &self.admission
    }
}

pub(crate) async fn capture(
    request: &ResolvedRequest,
    request_id: &str,
) -> Result<CapturedAdmission, ApiError> {
    capture_with_configuration(request, request_id)
        .await
        .map(|(admission, _, _)| admission)
}

/// Confirmed native actions must match the server's current origin/configuration.
/// An already accepted exact request may reconcile its first receipt after a
/// configuration change; the operation owner still checks identical inputs.
pub(crate) async fn capture_checked(
    repo: &argos_osint_core::storage::repo::Repository,
    request: &ResolvedRequest,
    request_id: &str,
    operation: &str,
    expected: Option<&fleetworks_contracts::context::MutationContext>,
) -> Result<CapturedAdmission, ApiError> {
    let expected = expected.ok_or(ApiError(
        axum::http::StatusCode::PRECONDITION_REQUIRED,
        "Confirmed origin and configuration preconditions required",
    ))?;
    let context = request.registered()?;
    if expected.registration_id != context.registration_id
        || expected.expected_registration_revision != context.registration_revision
        || expected.expected_context_revision != context.context_revision
    {
        return Err(ApiError(
            axum::http::StatusCode::CONFLICT,
            "Request origin changed; refresh confirmation",
        ));
    }
    let revision = expected.expected_configuration_revision.ok_or(ApiError(
        axum::http::StatusCode::PRECONDITION_REQUIRED,
        "Configuration revision required",
    ))?;
    let admission = capture(request, request_id).await?;
    if revision != admission.origin().configuration_revision
        && admission
            .replay_job(repo, operation)
            .await
            .map_err(crate::internal)?
            .is_none()
    {
        return Err(ApiError(
            axum::http::StatusCode::CONFLICT,
            "Configuration changed; refresh confirmation",
        ));
    }
    Ok(admission)
}

pub(crate) async fn capture_with_configuration(
    request: &ResolvedRequest,
    request_id: &str,
) -> Result<
    (
        CapturedAdmission,
        SettingsFile,
        argos_osint_core::secrets::AuthFile,
    ),
    ApiError,
> {
    if !(1..=128).contains(&request_id.len())
        || !request_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_".contains(&byte))
    {
        return Err(crate::bad());
    }
    let context = request.registered()?.clone();
    if context.access_epoch != request.access_epoch() {
        return Err(crate::request_context::ContextFailure::forbidden().into());
    }
    let source = request.source().ok_or_else(crate::bad)?;
    let (snapshot, lock) = tokio::task::spawn_blocking(|| {
        let root = configuration::workspace()?;
        let lock = ConfigLock::acquire_at(&root)
            .map_err(|_| crate::internal("configuration capture unavailable"))?;
        let snapshot = configuration::read_locked(&root, &lock)?;
        Ok::<_, ApiError>((snapshot, lock))
    })
    .await
    .map_err(|_| crate::internal("configuration capture task failed"))??;
    let mut references = snapshot.credential_references();
    references.push(format!("gateway:{}", request.credential_id()));
    let origin = JobOrigin::new(
        &context,
        source,
        request_id.to_owned(),
        Utc::now().to_rfc3339(),
        snapshot.revision,
        references,
    );
    let admission = match request.proof() {
        AdmissionProof::Native { .. } => {
            InteractiveAdmission::native(request.authority().clone(), context, origin)
        }
        AdmissionProof::Web {
            session_token,
            expected_context_revision,
        } => InteractiveAdmission::web(
            request.authority().clone(),
            context,
            origin,
            session_token.expose_secret().to_owned(),
            *expected_context_revision,
        ),
        AdmissionProof::None => return Err(crate::bad()),
    }
    .map_err(crate::internal)?;
    Ok((
        CapturedAdmission {
            admission,
            _configuration_lock: lock,
        },
        snapshot.settings,
        snapshot.auth,
    ))
}

#[cfg(test)]
mod tests;
