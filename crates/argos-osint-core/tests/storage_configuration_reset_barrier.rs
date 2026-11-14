//! Explicit jobs-only compatibility and fail-closed partial network authority.
#![cfg(feature = "surreal")]

mod support;

use argos_osint_core::storage::configuration_reset;
use argos_osint_core::storage::connect_registry;
use argos_osint_core::storage::jobs;
use argos_osint_core::storage::jobs::NewJob;
use chrono::Utc;
use support::Server;

fn job(id: &str) -> NewJob<'_> {
    NewJob {
        id,
        kind: "task",
        operation: "fixture_reset_barrier",
        parent: None,
        created_at: Utc::now(),
    }
}

#[tokio::test]
async fn jobs_only_adapter_is_explicit_and_partial_or_malformed_network_authority_cannot_admit_work(
) {
    let Some(server) = Server::start() else {
        return;
    };
    let db = server.raw_root("fixture", "reset_barrier").await;
    jobs::apply_schema(&db)
        .await
        .expect("intentional jobs-only adapter");
    jobs::create_job(&db, &job("legacy-allowed"), &[])
        .await
        .expect("explicit schema-only compatibility");
    db.query(connect_registry::SCHEMA)
        .await
        .expect("partial network metadata")
        .check()
        .expect("registry schema statements");
    assert!(
        jobs::create_job(&db, &job("partial-denied"), &[])
            .await
            .is_err(),
        "a partial network graph cannot fall back to the legacy adapter"
    );
    db.query(configuration_reset::SCHEMA)
        .await
        .expect("full reset metadata")
        .check()
        .expect("reset schema statements");
    jobs::create_job(&db, &job("complete-metadata-allowed"), &[])
        .await
        .expect("empty unfenced network metadata");
    // The deliberately malformed fixture changes only this disposable schema.
    // Typed metadata decoding must reject missing booleans, not filter the row
    // away as if no reset existed. No caller/transport error can permit work.
    db.query("REMOVE FIELD first_use_blocked ON TABLE configuration_reset_scope; CREATE configuration_reset_scope:malformed SET scope_id='fixture',active_reset='fixture-reset',access_epoch='fixture-access',revoked_json='[]';")
        .await.expect("synthetic malformed metadata").check().expect("malformed fixture statements");
    assert!(jobs::create_job(&db, &job("malformed-denied"), &[])
        .await
        .is_err());
    let mut retained = db
        .query(
            "SELECT VALUE record::id(id) FROM job WHERE operation='fixture_reset_barrier' ORDER BY id",
        )
        .await
        .expect("finite retained jobs read")
        .check()
        .expect("retained statement");
    let ids: Vec<String> = retained.take(0).expect("exact admitted job identities");
    assert_eq!(
        ids,
        vec!["complete-metadata-allowed", "legacy-allowed"],
        "denied creations roll back atomically"
    );
}
