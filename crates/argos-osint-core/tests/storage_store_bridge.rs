//! The synchronous `store_bridge` / `jobs_store` entry points the frozen
//! `Store` methods dispatch to, on the process runtime in the mode picked by
//! `ARGOS_SURREAL_MODE` (embedded) or `ARGOS_SURREAL_BIN` (server). One test
//! per process: the runtime is initialized once.
#![cfg(feature = "surreal")]

use argos_osint_core::memory_metadata::MemoryKind;
use argos_osint_core::storage::home::{self, Layout, Mode, StorageConfig};
use argos_osint_core::storage::{jobs_store, runtime, store_bridge};
use argos_osint_core::tasks::{JobMeta, NewJob};

fn mode() -> Option<Mode> {
    if std::env::var("ARGOS_SURREAL_MODE").as_deref() == Ok("embedded") {
        return cfg!(feature = "embedded").then_some(Mode::Embedded);
    }
    std::env::var_os("ARGOS_SURREAL_BIN").map(|_| Mode::Server)
}

fn seed(sql: &'static str) {
    runtime::blocking(|repo| async move {
        repo.db().query(sql).await?.check()?;
        Ok(())
    })
    .unwrap();
}

fn checks() {
    assert!(store_bridge::on());

    // register_job / set_job_progress against the job schema.
    let job = NewJob {
        id: "bridge-job".into(),
        kind: "task".into(),
        owner_scope: String::new(),
        input_revision: String::new(),
        deadline_at: String::new(),
    };
    let meta = JobMeta {
        app: "atlas".into(),
        title: "Bridge".into(),
        ..JobMeta::default()
    };
    jobs_store::register_job(&job, &meta).unwrap();
    jobs_store::register_job(&job, &meta).unwrap();
    jobs_store::set_job_progress("bridge-job", "running", "fetch", 2, Some(5), "").unwrap();
    let row = jobs_store::get_job("bridge-job").unwrap().unwrap();
    assert_eq!(
        (row.app.as_str(), row.state.as_str(), row.phase.as_str()),
        ("atlas", "running", "fetch")
    );
    assert_eq!((row.progress_done, row.progress_total), (Some(2), Some(5)));
    assert!(row.finished_at.is_empty());
    jobs_store::set_job_progress("bridge-job", "failed", "fetch", 2, None, "boom").unwrap();
    let row = jobs_store::get_job("bridge-job").unwrap().unwrap();
    assert_eq!((row.state.as_str(), row.progress_total), ("failed", None));
    assert_eq!(row.error_summary, "boom");
    assert!(!row.finished_at.is_empty());
    // A missing job is a no-op, not an error.
    jobs_store::set_job_progress("missing-job", "running", "", 0, None, "").unwrap();

    // SELECT ... FROM ONLY on missing records.
    assert_eq!(
        store_bridge::memory_kind("missing").unwrap(),
        MemoryKind::Claim
    );
    assert_eq!(store_bridge::actor_resolve("ghost").unwrap(), "ghost");
    assert_eq!(store_bridge::actor_display_name("ghost").unwrap(), None);
    assert!(store_bridge::actor_mentions("ghost").unwrap().is_empty());

    // related_endpoint_candidates: the other article endpoint of Related
    // memories, filtered by lifecycle.
    seed(
        "CREATE memory:rel_a CONTENT { text: 'a', memory_kind: 'related', lifecycle: 'active' };
         CREATE memory:rel_b CONTENT { text: 'b', memory_kind: 'related', lifecycle: 'superseded' };
         CREATE memory:rel_c CONTENT { text: 'c', memory_kind: 'related', lifecycle: 'retired' };
         CREATE memory:claim CONTENT { text: 'd', memory_kind: 'claim' };
         FOR $m IN ['rel_a', 'rel_b', 'rel_c', 'claim'] {
           CREATE brain_memory_endpoint CONTENT { memory_id: $m, side: 'source',
             endpoint_kind: 'article', origin: 'run', ref_id: 'art' };
           CREATE brain_memory_endpoint CONTENT { memory_id: $m, side: 'target',
             endpoint_kind: 'article', origin: 'run2', ref_id: string::concat('other_', $m) };
         };",
    );
    assert_eq!(
        store_bridge::related_endpoint_candidates("run", "art", true).unwrap(),
        vec![("run2".into(), "other_rel_a".into(), "rel_a".into())]
    );
    let mut loose = store_bridge::related_endpoint_candidates("run", "art", false).unwrap();
    loose.sort();
    assert_eq!(
        loose,
        vec![
            ("run2".into(), "other_rel_a".into(), "rel_a".into()),
            ("run2".into(), "other_rel_b".into(), "rel_b".into()),
        ]
    );
    // From the target side the source endpoint comes back.
    assert_eq!(
        store_bridge::related_endpoint_candidates("run2", "other_rel_a", true).unwrap(),
        vec![("run".into(), "art".into(), "rel_a".into())]
    );
    let ends = store_bridge::memory_article_endpoints("rel_a").unwrap();
    assert_eq!(ends.len(), 2);
    assert!(
        store_bridge::related_endpoint_candidates("nope", "art", false)
            .unwrap()
            .is_empty()
    );

    // Actor merges: redirect chain, display name, merged mentions.
    seed(
        "CREATE brain_actor:canon CONTENT { display_name: 'Canon', created_at: '', updated_at: '' };
         CREATE brain_actor_redirect:old CONTENT { to_actor: 'canon', created_at: '' };
         CREATE brain_actor_mention CONTENT { actor_id: 'old', origin_kind: 'article',
           origin_scope: 'run', origin_id: 'a1', created_at: '' };
         CREATE brain_actor_mention CONTENT { actor_id: 'canon', origin_kind: 'article',
           origin_scope: 'run', origin_id: 'a2', created_at: '' };",
    );
    assert_eq!(store_bridge::actor_resolve("old").unwrap(), "canon");
    assert_eq!(
        store_bridge::actor_display_name("old").unwrap().as_deref(),
        Some("Canon")
    );
    let m = store_bridge::actor_mentions("canon").unwrap();
    assert_eq!(m.len(), 2, "{m:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bridge_entry_points_on_the_process_runtime() {
    let Some(mode) = mode() else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let layout = Layout::new(dir.path());
    home::save_config(&layout, &StorageConfig { mode, port: None }).unwrap();
    runtime::init(layout.clone()).unwrap();
    std::thread::spawn(checks).join().unwrap();
}
