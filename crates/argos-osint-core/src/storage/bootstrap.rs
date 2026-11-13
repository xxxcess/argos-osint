//! SurrealDB schema bootstrap (P3 step 5), the counterpart of
//! `Store::ensure_schema`.
//!
//! [`ensure_schema`] applies every storage module's schema, in dependency
//! order, and records the schema version on `storage_meta:schema` in one
//! transaction. Analyzer definitions go first, as their own idempotent
//! batch outside that transaction: a full-text index defined in a
//! transaction cannot resolve an analyzer defined in the same one when the
//! indexed table already holds rows (SurrealDB 3.x), which broke opening a
//! version-1 database with `The analyzer 'atlas_article_text' does not
//! exist`; fresh stores never saw it because their index has no rows to
//! build. The analyzers are additive catalog entries with no data effect,
//! so a refused (newer) database keeps at most those while the guarded
//! definitions still roll back. As in the legacy store (`user_version`), a
//! database written by a newer build is refused and the transaction rolls
//! back. Every definition is `IF NOT EXISTS`, so a re-run (or a concurrent
//! bootstrap, which retries on conflict) is a no-op. Version nine adds the
//! explicit atomic workstation-contract migration after definitions and before
//! the version checkpoint; it preserves domain records and historical origins.

use surrealdb::engine::any::Any;
use surrealdb::Surreal;

use super::jobs::{run, take};
use super::tx::{run_tx, RetryPolicy, TxError};

/// SurrealDB schema version this build writes. v2 adds the Atlas admission
/// staging and rejection-receipt tables plus the canonical `categories`
/// array, `admitted_at`, the `categories.*` membership index and the two
/// article full-text indexes — all additive `IF NOT EXISTS` definitions, so
/// opening a v1 database upgrades it in place without a wipe. v4 adds the
/// durable dispatch outbox for opt-in worker pools. v6 adds leased model account admission.
/// v8 includes native registry/origin/reset fences and durable actor rebuild state;
/// runtime readers reject incomplete earlier metadata until explicit bootstrap.
/// v9 migrates active workspace registrations to workstation-only identity atomically,
/// preserving immutable origins and fencing incompatible writers.
/// v10 adds durable native pairing, scoped installation credentials and revocation.
pub const SCHEMA_VERSION: i64 = 10;

/// Module schemas in apply order: counters before the outbox; the graph
/// before lifecycle (which indexes `brain_graph_node`); Atlas before its
/// metadata (whose event is on `atlas_article`); jobs, insight and intel
/// before refresh. The event logs (`argos_event`, `investigation_event`)
/// stand alone; telemetry writes use `profile_view`'s tables, with
/// `telemetry_meta` after them.
pub fn schemas() -> [(&'static str, &'static str); 57] {
    [
        ("counters", super::counters::SCHEMA),
        ("native_pairing", super::native_pairing::SCHEMA),
        ("outbox", super::outbox::SCHEMA),
        ("memory", super::memory::SCHEMA),
        ("graph", super::graph::SCHEMA),
        ("embeddings", super::embeddings::SCHEMA),
        ("processes", super::processes::SCHEMA),
        ("jobs", super::jobs::SCHEMA),
        ("insight", super::insight::SCHEMA),
        ("atlas", super::atlas::SCHEMA),
        ("atlas_metadata", super::atlas_metadata::SCHEMA),
        ("classifier_admission", super::classifier_admission::SCHEMA),
        (
            "classifier_calibration",
            super::classifier_calibration::SCHEMA,
        ),
        (
            "atlas_admission_maintenance",
            super::atlas_admission_maintenance::SCHEMA,
        ),
        ("classifier_review", super::classifier_review::SCHEMA),
        ("classifier_learning", super::classifier_learning::SCHEMA),
        ("atlas_derivatives", crate::atlas_derivative_worker::SCHEMA),
        (
            "atlas_derivative_status",
            super::atlas_derivative_status::SCHEMA,
        ),
        ("intel", super::intel::SCHEMA),
        ("brief", super::brief::SCHEMA),
        ("lifecycle", super::lifecycle::SCHEMA),
        ("actor_catalog", super::actor_catalog::SCHEMA),
        ("refresh", super::refresh::SCHEMA),
        ("recon", super::recon::SCHEMA),
        ("entities", super::entities::SCHEMA),
        ("recon_threads", super::recon_threads::SCHEMA),
        ("recon_runs", super::recon_runs::SCHEMA),
        ("recon_exec", super::recon_exec::SCHEMA),
        ("recon_jobs", super::recon_jobs::SCHEMA),
        ("recon_extract", super::recon_extract::SCHEMA),
        ("credits", super::credits::SCHEMA),
        ("pipeline_inputs", super::pipeline_inputs::SCHEMA),
        ("article_body", super::article_body::SCHEMA),
        ("nlp_cache", super::nlp_cache::SCHEMA),
        (
            "atlas_derivative_invalidation",
            super::atlas_derivative_invalidation::SCHEMA,
        ),
        ("pipeline_exec", super::pipeline_exec::SCHEMA),
        ("profile_view", super::profile_view::SCHEMA),
        ("provider_account", super::provider_account::SCHEMA),
        ("provider_pacer", super::provider_pacer::SCHEMA),
        ("telemetry", super::telemetry::SCHEMA),
        ("investigation", super::investigation::SCHEMA),
        ("atlas_work", super::atlas_work::SCHEMA),
        ("actor_index", super::lifecycle::ARTICLE_INDEX_SCHEMA),
        ("event_log", super::event_log::SCHEMA),
        ("summary_flush", super::summary_flush::SCHEMA),
        ("brain_scope", super::brain_scope::SCHEMA),
        ("brief_stages", super::brief_stages::SCHEMA),
        ("meta", META_SCHEMA),
        ("connect_registry", super::connect_registry::SCHEMA),
        ("cycle_history", super::cycle_history::SCHEMA),
        ("configuration_reset", super::configuration_reset::SCHEMA),
        ("job_context", super::job_context::SCHEMA),
        ("job_actions", super::job_actions::SCHEMA),
        ("job_dispatch", super::job_dispatch::SCHEMA),
        ("setup_jobs", super::setup_jobs::SCHEMA),
        (
            "classifier_activation",
            super::classifier_activation::SCHEMA,
        ),
        ("actor_search_outbox", super::actor_search_outbox::SCHEMA),
    ]
}

/// The version record (`storage_meta:schema`) and the database identity
/// (`storage_meta:identity`, see `storage::server`), and the reset epoch
/// (`storage_meta:epoch`, see `storage::commands`), and the serving
/// embedding generation (`storage_meta:vectors`, see `storage::vectors`).
pub const META_SCHEMA: &str = "DEFINE TABLE IF NOT EXISTS storage_meta SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS version ON storage_meta TYPE option<int>;
DEFINE FIELD IF NOT EXISTS applied_at ON storage_meta TYPE option<string>;
DEFINE FIELD IF NOT EXISTS instance_id ON storage_meta TYPE option<string>;
DEFINE FIELD IF NOT EXISTS reset_epoch ON storage_meta TYPE option<string>;
DEFINE FIELD IF NOT EXISTS reset_at ON storage_meta TYPE option<string>;
DEFINE FIELD IF NOT EXISTS embedding_generation ON storage_meta TYPE option<int>;
DEFINE FIELD IF NOT EXISTS embedding_model ON storage_meta TYPE option<string>;";

/// What [`ensure_schema`] found and did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bootstrap {
    /// Version recorded before this call (0: fresh database).
    pub previous: i64,
    pub version: i64,
}

/// Check the schema version using a database-scoped runtime session.
///
/// The runtime process does not write metadata or apply schema definitions.
pub async fn verify_schema(db: &Surreal<Any>) -> Result<Bootstrap, TxError> {
    let failed = |_| TxError::Abort("could not read the network storage schema".into());
    let mut result = db
        .query("SELECT VALUE version FROM ONLY storage_meta:schema")
        .await
        .and_then(|result| result.check())
        .map_err(failed)?;
    let version: Option<i64> = result.take(0).map_err(failed)?;
    match version {
        Some(version) if version == SCHEMA_VERSION => Ok(Bootstrap {
            previous: version,
            version,
        }),
        Some(version) => Err(TxError::Abort(format!(
            "network storage schema version {version} does not match this build ({SCHEMA_VERSION})"
        ))),
        None => Err(TxError::Abort(
            "network storage schema is missing. Run the storage bootstrap".into(),
        )),
    }
}

/// Retain the database identity across restarts. A fresh in-memory server
/// receives a new identity, which fences work enqueued for the previous store.
/// Establish the retained identity during explicit network bootstrap.
/// Readiness, observations and ordinary runtime requests must never call this.
pub async fn ensure_network_identity(db: &Surreal<Any>) -> Result<(), TxError> {
    let identity = crate::store::new_id();
    run_tx(db, RetryPolicy::default(), |tx| {
        let identity = identity.clone();
        async move {
            run(
                &tx,
                tx.query(
                    "UPSERT storage_meta:identity SET instance_id = instance_id ?? $new RETURN NONE",
                )
                .bind(("new", identity)),
            )
            .await?;
            Ok(())
        }
    })
    .await?;
    Ok(())
}

/// Apply every schema and record [`SCHEMA_VERSION`] in one transaction.
///
/// Analyzer definitions run before that transaction (see the module docs):
/// the guarded transaction still holds every table, field and index
/// definition together with the version read, the newer-than-build refusal
/// and the version write.
pub async fn ensure_schema(db: &Surreal<Any>) -> Result<Bootstrap, TxError> {
    for analyzers in analyzers() {
        db.query(analyzers).await?.check()?;
    }
    let mut sql = String::new();
    for (_, schema) in schemas() {
        sql.push_str(schema.trim().trim_end_matches(';'));
        sql.push_str(";\n");
    }
    let sql = std::sync::Arc::new(sql);
    let out = run_tx(db, RetryPolicy::default(), |tx| {
        let sql = sql.clone();
        async move {
            // Definitions first (tables must exist to be read); a refusal
            // below aborts the transaction, so they roll back with it.
            run(&tx, tx.query(sql.as_str())).await?;
            let mut r = run(
                &tx,
                tx.query("SELECT VALUE version FROM storage_meta WHERE id = storage_meta:schema"),
            )
            .await?;
            let previous = take::<Vec<i64>>(&mut r, 0)?.into_iter().next().unwrap_or(0);
            if previous > SCHEMA_VERSION {
                return Err(TxError::Abort(format!(
                    "database schema version {previous} is newer than this Argos build"
                )));
            }
            super::connect_registry::migrate_workstation_contract_in(&tx).await?;
            run(
                &tx,
                tx.query(
                    "UPSERT storage_meta:schema SET version = $v, applied_at = $now RETURN NONE",
                )
                .bind(("v", SCHEMA_VERSION))
                .bind(("now", chrono::Utc::now().to_rfc3339())),
            )
            .await?;
            Ok(Bootstrap {
                previous,
                version: SCHEMA_VERSION,
            })
        }
    })
    .await?;
    Ok(out.value)
}

/// Analyzer definitions from every module that has them, applied before the
/// guarded schema transaction (see the module docs). Modules keep theirs in
/// an `ANALYZERS` constant next to the full-text indexes that name them; a
/// module adding one must be wired in here.
fn analyzers() -> impl Iterator<Item = &'static str> {
    [super::atlas::ANALYZERS, super::recon_extract::ANALYZERS]
        .into_iter()
        .filter(|analyzers| !analyzers.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_schemas_hold_no_analyzer_definitions() {
        // An analyzer inside the guarded transaction cannot be resolved by
        // a full-text index over existing rows (see `ensure_schema`), so
        // every module keeps its analyzers in `ANALYZERS` instead.
        for (name, schema) in schemas() {
            assert!(
                !schema.to_ascii_uppercase().contains("DEFINE ANALYZER"),
                "{name}: analyzer definitions belong in the module's `ANALYZERS`, \
                 applied ahead of the guarded transaction"
            );
        }
        assert!(analyzers().count() >= 2, "analyzers went missing");
    }
}
