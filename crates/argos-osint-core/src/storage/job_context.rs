//! Immutable interactive job origin and durable requester membership.
//!
//! Admission calls these helpers inside its existing job transaction. The
//! registry guard is revalidated in that same transaction, fencing retirement
//! against newly accepted work. Reuse adds a request association without
//! rewriting a creator; legacy jobs never gain an invented original creator.

use fleetworks_contracts::context::ActivityScope;
use fleetworks_contracts::context::JobOrigin;
use fleetworks_contracts::context::OriginSource;
use fleetworks_contracts::context::RegisteredContext;
use sha2::Digest;
use sha2::Sha256;
use surrealdb::types::SurrealValue;

use super::connect_registry;
use super::connect_registry::RegistryAuthority;
use super::jobs;
use super::repo::Repository;
use super::tx::Tx;
use super::tx::TxError;

/// Additive metadata only; existing jobs/domain history are retained.
pub const SCHEMA: &str = "
DEFINE TABLE IF NOT EXISTS job_origin SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS job_id ON job_origin TYPE string;
DEFINE FIELD IF NOT EXISTS root_id ON job_origin TYPE string;
DEFINE FIELD IF NOT EXISTS scope_id ON job_origin TYPE string;
DEFINE FIELD IF NOT EXISTS deployment_id ON job_origin TYPE string;
DEFINE FIELD IF NOT EXISTS workstation_id ON job_origin TYPE string;
DEFINE FIELD IF NOT EXISTS workspace_id ON job_origin TYPE string;
DEFINE FIELD IF NOT EXISTS origin_json ON job_origin TYPE string;
DEFINE INDEX IF NOT EXISTS job_origin_job ON job_origin FIELDS job_id UNIQUE;
DEFINE INDEX IF NOT EXISTS job_origin_root ON job_origin FIELDS root_id;
DEFINE TABLE IF NOT EXISTS job_request SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS request_key ON job_request TYPE string;
DEFINE FIELD IF NOT EXISTS operation ON job_request TYPE string;
DEFINE FIELD IF NOT EXISTS job_id ON job_request TYPE string;
DEFINE FIELD IF NOT EXISTS scope_id ON job_request TYPE string;
DEFINE FIELD IF NOT EXISTS deployment_id ON job_request TYPE string;
DEFINE FIELD IF NOT EXISTS registration_id ON job_request TYPE string;
DEFINE FIELD IF NOT EXISTS workstation_id ON job_request TYPE string;
DEFINE FIELD IF NOT EXISTS workspace_id ON job_request TYPE string;
DEFINE FIELD IF NOT EXISTS requested_at ON job_request TYPE string;
DEFINE FIELD IF NOT EXISTS origin_json ON job_request TYPE string;
DEFINE INDEX IF NOT EXISTS job_request_identity ON job_request FIELDS request_key UNIQUE;
DEFINE INDEX IF NOT EXISTS job_request_node ON job_request FIELDS scope_id, deployment_id, workstation_id, requested_at;
DEFINE INDEX IF NOT EXISTS job_request_child ON job_request FIELDS scope_id, deployment_id, workstation_id, workspace_id, requested_at;
DEFINE INDEX IF NOT EXISTS job_request_job ON job_request FIELDS job_id, scope_id, workstation_id;
";

/// Trusted adapter-owned interactive origin and the proof rechecked at commit.
///
/// Never deserialize this type from HTTP or place its session proof in a job
/// payload. Durable jobs store only the immutable, non-secret origin snapshot.
#[derive(Clone)]
pub struct InteractiveAdmission {
    authority: RegistryAuthority,
    context: RegisteredContext,
    origin: JobOrigin,
    web_proof: Option<(String, u64)>,
}

impl InteractiveAdmission {
    /// Construct native admission after the gateway resolves the registration.
    pub fn native(
        authority: RegistryAuthority,
        context: RegisteredContext,
        origin: JobOrigin,
    ) -> Result<Self, TxError> {
        if origin.source != OriginSource::Native {
            return Err(abort("invalid native source"));
        }
        Ok(Self {
            authority,
            context,
            origin,
            web_proof: None,
        })
    }

    /// Keep opaque browser proof only until the actual admission transaction.
    pub fn web(
        authority: RegistryAuthority,
        context: RegisteredContext,
        origin: JobOrigin,
        token: String,
        context_revision: u64,
    ) -> Result<Self, TxError> {
        if origin.source != OriginSource::IntelWeb
            || token.len() != 64
            || !token.bytes().all(|byte| byte.is_ascii_hexdigit())
            || context_revision == 0
        {
            return Err(abort("invalid browser source proof"));
        }
        Ok(Self {
            authority,
            context,
            origin,
            web_proof: Some((token, context_revision)),
        })
    }

    pub fn authority(&self) -> &RegistryAuthority {
        &self.authority
    }
    pub fn context(&self) -> &RegisteredContext {
        &self.context
    }
    pub fn origin(&self) -> &JobOrigin {
        &self.origin
    }

    /// Reconcile a previously accepted request under its current authority.
    /// The original creator/configuration snapshot is never rewritten.
    pub async fn replay_job(
        &self,
        repo: &Repository,
        operation: &str,
    ) -> Result<Option<String>, TxError> {
        let key = request_key(&self.context, operation, &self.origin.request_id);
        super::tx::run_tx(repo.db(), super::tx::RetryPolicy::default(), |tx| {
            let key = &key;
            async move {
                let mut response = jobs::run(
                    &tx,
                    tx.query("SELECT job_id FROM ONLY $record")
                        .bind(("record", jobs::rid("job_request", &key))),
                )
                .await?;
                let saved: Option<SavedRequest> = jobs::take(&mut response, 0)?;
                if let Some(saved) = saved {
                    self.record_in(&tx, &saved.job_id, operation, false).await?;
                    Ok(Some(saved.job_id))
                } else {
                    Ok(None)
                }
            }
        })
        .await
        .map(|committed| committed.value)
    }

    pub(crate) fn web_proof(&self) -> Option<(&str, u64)> {
        self.web_proof
            .as_ref()
            .map(|(token, revision)| (token.as_str(), *revision))
    }

    pub(crate) async fn record_in(
        &self,
        tx: &Tx,
        job_id: &str,
        operation: &str,
        created: bool,
    ) -> Result<(), TxError> {
        match self.web_proof() {
            Some((token, revision)) => {
                record_web_admission_in(
                    tx,
                    &self.authority,
                    &self.context,
                    &self.origin,
                    token,
                    revision,
                    job_id,
                    operation,
                    created,
                )
                .await
            }
            None => {
                record_admission_in(
                    tx,
                    &self.authority,
                    &self.context,
                    &self.origin,
                    job_id,
                    operation,
                    created,
                )
                .await
            }
        }
    }
}

#[derive(SurrealValue)]
struct SavedRequest {
    job_id: String,
}

#[derive(SurrealValue)]
struct SavedOrigin {
    root_id: String,
    scope_id: String,
    deployment_id: String,
    workstation_id: String,
    workspace_id: String,
    origin_json: String,
}

#[derive(SurrealValue)]
struct SavedLineage {
    parent_id: Option<String>,
}

fn abort(message: &str) -> TxError {
    TxError::Abort(message.into())
}

fn opaque(value: &str, maximum: usize) -> bool {
    !value.is_empty() && value.len() <= maximum && !value.chars().any(char::is_control)
}

fn request_key(context: &RegisteredContext, operation: &str, request_id: &str) -> String {
    request_key_for(
        &context.deployment_id,
        &context.registration_id,
        operation,
        request_id,
    )
}

fn request_key_for(
    deployment: &str,
    registration: &str,
    operation: &str,
    request_id: &str,
) -> String {
    let mut hash = Sha256::new();
    for value in [deployment, registration, operation, request_id] {
        hash.update((value.len() as u64).to_le_bytes());
        hash.update(value.as_bytes());
    }
    format!("{:x}", hash.finalize())
}

#[derive(SurrealValue)]
struct SavedAssociation {
    job_id: String,
    operation: String,
    scope_id: String,
    deployment_id: String,
    registration_id: String,
    workstation_id: String,
    workspace_id: String,
    requested_at: String,
    origin_json: String,
}

/// Reconcile an exact accepted request without submitting work or changing history.
/// The adapter must validate its current proof and activity permission first.
/// An old registration ID is a lookup reference only; saved scope/node/child
/// ownership must still match the current authorized context before any reply.
pub async fn accepted_request(
    repo: &Repository,
    authority: &RegistryAuthority,
    context: &RegisteredContext,
    registration: &str,
    operation: &str,
    request_id: &str,
) -> Result<Option<fleetworks_contracts::context::JobRequestAssociation>, TxError> {
    if !opaque(registration, 128) || !opaque(operation, 64) || !opaque(request_id, 128) {
        return Err(abort("invalid request receipt reference"));
    }
    let key = request_key_for(&context.deployment_id, registration, operation, request_id);
    let mut response = repo
        .db()
        .query("SELECT * FROM ONLY $request")
        .bind(("request", jobs::rid("job_request", &key)))
        .await?
        .check()?;
    let saved: Option<SavedAssociation> = response.take(0)?;
    let Some(saved) = saved else {
        return Ok(None);
    };
    if saved.scope_id != authority.scope_id()
        || saved.deployment_id != context.deployment_id
        || saved.workstation_id != context.workstation_id
        || saved.workspace_id != context.workspace_id
        || saved.registration_id != registration
        || saved.operation != operation
    {
        return Ok(None);
    }
    if saved.origin_json.len() > 32 * 1024 {
        return Err(abort("invalid bounded request receipt"));
    }
    let origin: JobOrigin =
        serde_json::from_str(&saved.origin_json).map_err(|_| abort("invalid request receipt"))?;
    if origin.request_id != request_id
        || origin.registration_id != registration
        || origin.deployment_id != context.deployment_id
        || origin.workspace_id != context.workspace_id
        || origin.workstation_id != context.workstation_id
    {
        return Err(abort("inconsistent request receipt"));
    }
    Ok(Some(
        fleetworks_contracts::context::JobRequestAssociation::new(
            saved.job_id,
            origin,
            saved.requested_at,
        ),
    ))
}

/// Persist attribution alongside durable acceptance or shared-job reuse.
///
/// `origin` must be constructed from effective server configuration and the
/// authenticated context, never decoded as public admission authority. The
/// registration/admission/configuration epochs are rechecked under the registry
/// guard. A replay retains its first captured origin, including submission time.
pub(crate) async fn record_admission_in(
    tx: &Tx,
    authority: &RegistryAuthority,
    context: &RegisteredContext,
    origin: &JobOrigin,
    job_id: &str,
    operation: &str,
    created: bool,
) -> Result<(), TxError> {
    if origin.source != OriginSource::Native {
        return Err(abort("web admission requires a current session proof"));
    }
    record_admission_inner(tx, authority, context, origin, job_id, operation, created).await
}

/// Revalidate browser proof in the actual logout/retirement transaction fence.
pub(crate) async fn record_web_admission_in(
    tx: &Tx,
    authority: &RegistryAuthority,
    context: &RegisteredContext,
    origin: &JobOrigin,
    session_token: &str,
    expected_context_revision: u64,
    job_id: &str,
    operation: &str,
    created: bool,
) -> Result<(), TxError> {
    if origin.source != OriginSource::IntelWeb {
        return Err(abort("invalid browser admission source"));
    }
    let session = connect_registry::validate_session_for_admission_in(
        tx,
        authority,
        session_token,
        expected_context_revision,
    )
    .await?;
    if !session.context.same_request_context(context) {
        return Err(abort("browser admission context changed; reconnect"));
    }
    record_admission_inner(tx, authority, context, origin, job_id, operation, created).await
}

async fn record_admission_inner(
    tx: &Tx,
    authority: &RegistryAuthority,
    context: &RegisteredContext,
    origin: &JobOrigin,
    job_id: &str,
    operation: &str,
    created: bool,
) -> Result<(), TxError> {
    if !opaque(job_id, 512)
        || !opaque(operation, 64)
        || !opaque(&origin.request_id, 128)
        || origin.credential_references.len() > 64
        || origin
            .credential_references
            .iter()
            .any(|reference| !opaque(reference, 256))
    {
        return Err(abort("invalid durable request identity"));
    }
    // Share the caller's registry guard and actual acceptance transaction.
    super::configuration_reset::check_user_epoch_in(
        tx,
        authority.scope_id(),
        authority.access_epoch(),
        None,
    )
    .await?;
    let registration = connect_registry::validate_registration_in(
        tx,
        authority,
        &context.registration_id,
        context.registration_revision,
    )
    .await
    .map_err(|_| abort("registered admission context unavailable or changed"))?;
    let state = connect_registry::deployment_state_in(tx)
        .await
        .map_err(|_| abort("registered admission authority unavailable"))?;
    if !state.admission_open
        || state.deployment_id != context.deployment_id
        || state.configuration_epoch != context.configuration_epoch
        || authority.access_epoch() != context.access_epoch
        || registration.workstation_id != context.workstation_id
        || registration.workspace_id != context.workspace_id
        || registration.connect_id != context.connect_id
        || origin.deployment_id != context.deployment_id
        || origin.registration_id != context.registration_id
        || origin.workstation_id != context.workstation_id
        || origin.workspace_id != context.workspace_id
        || origin.connect_id_at_submission != context.connect_id
        || origin.workstation_ip_at_submission != context.workstation_ip
        || origin.workspace_name_at_submission != context.workspace_name
        || origin.configuration_epoch != context.configuration_epoch
    {
        return Err(abort("registered admission context unavailable or changed"));
    }
    let key = request_key(context, operation, &origin.request_id);
    let mut response = jobs::run(
        tx,
        tx.query("RETURN record::exists($job)")
            .bind(("job", jobs::rid("job", job_id))),
    )
    .await?;
    if jobs::take::<Option<bool>>(&mut response, 0)? != Some(true) {
        return Err(abort("durable request job is unavailable"));
    }
    let request_record = jobs::rid("job_request", &key);
    let mut response = jobs::run(
        tx,
        tx.query("SELECT job_id FROM ONLY $record")
            .bind(("record", request_record.clone())),
    )
    .await?;
    let existing: Option<SavedRequest> = jobs::take(&mut response, 0)?;
    if let Some(existing) = existing {
        if existing.job_id != job_id {
            return Err(abort(
                "request identity already accepted for different work",
            ));
        }
        return Ok(());
    }
    let origin_json =
        serde_json::to_string(origin).map_err(|_| abort("invalid non-secret origin snapshot"))?;
    if created {
        jobs::run(tx, tx.query(
            "CREATE $record SET job_id = $job, root_id = $job, scope_id = $scope, \
             deployment_id = $deployment, workstation_id = $node, workspace_id = $workspace, origin_json = $origin RETURN NONE",
        ).bind(("record", jobs::rid("job_origin", job_id)))
            .bind(("job", job_id.to_owned()))
            .bind(("scope", authority.scope_id().to_owned()))
            .bind(("deployment", context.deployment_id.clone()))
            .bind(("node", context.workstation_id.clone()))
            .bind(("workspace", context.workspace_id.clone()))
            .bind(("origin", origin_json.clone()))).await?;
    }
    jobs::run(tx, tx.query(
        "CREATE $record SET request_key = $key, operation = $operation, job_id = $job, \
         scope_id = $scope, deployment_id = $deployment, registration_id = $registration, \
         workstation_id = $node, workspace_id = $workspace, requested_at = $at, origin_json = $origin RETURN NONE",
    ).bind(("record", request_record))
        .bind(("key", key))
        .bind(("operation", operation.to_owned()))
        .bind(("job", job_id.to_owned()))
        .bind(("scope", authority.scope_id().to_owned()))
        .bind(("deployment", context.deployment_id.clone()))
        .bind(("registration", context.registration_id.clone()))
        .bind(("node", context.workstation_id.clone()))
        .bind(("workspace", context.workspace_id.clone()))
        .bind(("at", origin.submitted_at.clone()))
        .bind(("origin", origin_json))).await?;
    Ok(())
}

/// Carry a genuine causal relationship without copying an unbounded requester list.
pub(crate) async fn record_child_in(tx: &Tx, parent: &str, child: &str) -> Result<(), TxError> {
    let mut lineage = jobs::run(
        tx,
        tx.query(
            "SELECT record::id(parent) AS parent_id FROM ONLY $job; SELECT * FROM ONLY $origin;",
        )
        .bind(("job", jobs::rid("job", child)))
        .bind(("origin", jobs::rid("job_origin", child))),
    )
    .await?;
    let actual: Option<SavedLineage> = jobs::take(&mut lineage, 0)?;
    if !actual.is_some_and(|lineage| lineage.parent_id.as_deref() == Some(parent)) {
        return Err(abort("child origin requires genuine job parent"));
    }
    let existing: Option<SavedOrigin> = jobs::take(&mut lineage, 1)?;
    let mut response = jobs::run(
        tx,
        tx.query("SELECT * FROM ONLY $record")
            .bind(("record", jobs::rid("job_origin", parent))),
    )
    .await?;
    let parent: Option<SavedOrigin> = jobs::take(&mut response, 0)?;
    let Some(parent) = parent else {
        // Unattributed/system work stays unattributed rather than being assigned
        // an arbitrary workstation for visibility.
        return Ok(());
    };
    connect_registry::fence_causal_child_in(tx, &parent.scope_id, &parent.workspace_id).await?;
    if let Some(existing) = existing {
        if existing.root_id == parent.root_id
            && existing.origin_json == parent.origin_json
            && existing.scope_id == parent.scope_id
        {
            return Ok(());
        }
        return Err(abort("child origin is immutable"));
    }
    jobs::run(tx, tx.query(
        "CREATE $record SET job_id = $job, root_id = $root, scope_id = $scope, \
         deployment_id = $deployment, workstation_id = $node, workspace_id = $workspace, origin_json = $origin RETURN NONE",
    ).bind(("record", jobs::rid("job_origin", child)))
        .bind(("job", child.to_owned()))
        .bind(("root", parent.root_id))
        .bind(("scope", parent.scope_id))
        .bind(("deployment", parent.deployment_id))
        .bind(("node", parent.workstation_id))
        .bind(("workspace", parent.workspace_id))
        .bind(("origin", parent.origin_json))).await?;
    Ok(())
}

/// Bounded requester-associated top-level IDs, before loading any job payload.
///
/// The gateway must first authorize the selected node/current access epoch and
/// optional child. This function never falls back to unscoped history.
pub async fn requested_job_ids(
    repo: &Repository,
    authority_scope: &str,
    scope: &ActivityScope,
    limit: usize,
) -> anyhow::Result<Vec<String>> {
    anyhow::ensure!(
        opaque(authority_scope, 128)
            && opaque(&scope.deployment_id, 128)
            && opaque(&scope.workstation_id, 128),
        "workstation activity scope required"
    );
    let mut response = repo.db().query(
        "SELECT job_id, requested_at FROM job_request WHERE scope_id = $scope AND deployment_id = $deployment \
         AND workstation_id = $node AND ($workspace = NONE OR workspace_id = $workspace) \
         ORDER BY requested_at DESC, job_id ASC LIMIT $limit",
    ).bind(("scope", authority_scope.to_owned()))
        .bind(("deployment", scope.deployment_id.clone()))
        .bind(("node", scope.workstation_id.clone()))
        .bind(("workspace", scope.workspace_id.clone()))
        .bind(("limit", limit.clamp(1, 200) as i64)).await?.check()?;
    let rows: Vec<SavedRequest> = response.take(0)?;
    // Reused requests may name one job more than once. Deduplicate only after
    // the bounded membership query; this does not claim exhaustive pagination.
    let mut seen = std::collections::BTreeSet::new();
    Ok(rows
        .into_iter()
        .map(|row| row.job_id)
        .filter(|id| seen.insert(id.clone()))
        .collect())
}

/// Read immutable provenance for a trusted worker's already claimed job.
/// This is not an HTTP authorization check; adapters authorize before exposing it.
pub async fn origin_for_job(repo: &Repository, job_id: &str) -> Result<Option<JobOrigin>, TxError> {
    if !opaque(job_id, 512) {
        return Err(abort("invalid job origin reference"));
    }
    let mut response = repo
        .db()
        .query("SELECT * FROM ONLY $origin")
        .bind(("origin", jobs::rid("job_origin", job_id)))
        .await?
        .check()?;
    let saved: Option<SavedOrigin> = response.take(0)?;
    saved
        .map(|saved| {
            if saved.origin_json.len() > 32 * 1024 {
                return Err(abort("invalid bounded origin snapshot"));
            }
            serde_json::from_str(&saved.origin_json).map_err(|_| abort("invalid origin snapshot"))
        })
        .transpose()
}

/// Check a direct job/causal-child ID before details, results, exports or live.
pub async fn permits_job(
    repo: &Repository,
    authority_scope: &str,
    scope: &ActivityScope,
    job_id: &str,
) -> anyhow::Result<bool> {
    anyhow::ensure!(
        opaque(authority_scope, 128)
            && opaque(job_id, 512)
            && opaque(&scope.deployment_id, 128)
            && opaque(&scope.workstation_id, 128),
        "workstation activity scope required"
    );
    let mut response = repo
        .db()
        .query(
            "LET $root = (SELECT VALUE root_id FROM ONLY $origin) ?? $job; \
         SELECT VALUE count() FROM job_request WHERE job_id = $root AND scope_id = $scope \
         AND deployment_id = $deployment AND workstation_id = $node \
         AND ($workspace = NONE OR workspace_id = $workspace) GROUP ALL",
        )
        .bind(("origin", jobs::rid("job_origin", job_id)))
        .bind(("job", job_id.to_owned()))
        .bind(("scope", authority_scope.to_owned()))
        .bind(("deployment", scope.deployment_id.clone()))
        .bind(("node", scope.workstation_id.clone()))
        .bind(("workspace", scope.workspace_id.clone()))
        .await?
        .check()?;
    let counts: Vec<i64> = response.take(1)?;
    Ok(counts.first().copied().unwrap_or(0) > 0)
}

/// True only after actual associated active work is absent under the same
/// registry/admission transaction guard; missing schema/query errors fail closed.
pub(crate) async fn has_active_workspace_work_in(
    tx: &Tx,
    scope_id: &str,
    deployment_id: &str,
    workspace_id: &str,
) -> Result<bool, TxError> {
    // A partially installed/missing attribution schema is not evidence that a
    // workspace has no work. INFO fails before a missing table can look empty.
    jobs::run(
        tx,
        tx.query("INFO FOR TABLE job_request; INFO FOR TABLE job_origin; INFO FOR TABLE classifier_activation_operation;"),
    )
    .await?;
    let mut response = jobs::run(tx, tx.query(
        "SELECT VALUE job_id FROM job_request WHERE scope_id=$scope AND deployment_id=$deployment \
         AND workspace_id=$workspace AND count(SELECT VALUE id FROM job \
           WHERE (record::id(id)=$parent.job_id OR record::id(root)=$parent.job_id) \
           AND (state NOT IN ['completed','succeeded','failed','partial','cancelled','superseded','stopped'] \
             OR count(SELECT VALUE id FROM task WHERE job=$parent.id \
               AND state NOT IN ['completed','succeeded','failed','cancelled','superseded','skipped','stopped'] LIMIT 1)>0) LIMIT 1)>0 LIMIT 1",
    ).bind(("scope", scope_id.to_owned()))
        .bind(("deployment", deployment_id.to_owned()))
        .bind(("workspace", workspace_id.to_owned()))).await?;
    let active: Vec<String> = jobs::take(&mut response, 0)?;
    if !active.is_empty() {
        return Ok(true);
    }
    // Activation creates its own attributed root. Its generic job may have
    // failed before an old worker's filesystem effect became provably settled.
    // Only the existing durable requester association can pin this workspace.
    let mut uncertain = jobs::run(tx, tx.query(
        "SELECT VALUE job_id FROM job_request WHERE scope_id=$scope AND deployment_id=$deployment \
         AND workspace_id=$workspace \
         AND type::record('classifier_activation_operation',job_id).scope_id=$scope \
         AND type::record('classifier_activation_operation',job_id).state IN ['running','effect_uncertain'] LIMIT 1",
    ).bind(("scope",scope_id.to_owned())).bind(("deployment",deployment_id.to_owned())).bind(("workspace",workspace_id.to_owned()))).await?;
    Ok(!jobs::take::<Vec<String>>(&mut uncertain, 0)?.is_empty())
}
