//! The brief chain on embedded SurrealDB (`storage::brief_stages`), with
//! the SQLite suite's mock backend.

use super::*;
use crate::storage::brief_stages as surreal_stages;
use crate::storage::jobs;
use crate::storage::open::Target;
use crate::storage::repo::Repository;
use crate::storage::tx::{run_tx, RetryPolicy};

async fn repo(dir: &tempfile::TempDir) -> Repository {
    let repo = Repository::connect(
        &Target::Embedded {
            path: dir.path().join("surreal"),
        },
        "argos",
        "brief_stages",
    )
    .await
    .unwrap();
    let db = repo.db();
    crate::storage::atlas::insert_run(db, "fixture", "{}", "{}")
        .await
        .unwrap();
    crate::storage::atlas::upsert_article(
        db,
        &article("fixture", "a1", TITLE, DESC, "https://example.test/a1"),
    )
    .await
    .unwrap();
    repo
}

// §8.5 on SurrealDB (`sqlite_contention_and_publication_delay_keep_leases_and_commit_once`):
// a competing writer keeps conflicting with the chain's guard and task rows
// while slow stages run and commit; renewal and commit retry within their
// bounds, no stage is reaped, and every stage commits exactly once.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn surreal_contention_and_publication_delay_keep_leases_and_commit_once() {
    let dir = tempfile::tempdir().unwrap();
    let repo = repo(&dir).await;
    let chain = surreal_stages::ensure(&repo, &inputs("r1")).await.unwrap();
    assert!(chain.created);
    let job = chain.job_id.clone();
    let tasks: Vec<String> = brief_pipeline::STAGES
        .iter()
        .map(|(op, _)| surreal_stages::task_id(&job, op))
        .collect();
    let mut mock = Mock::happy();
    mock.delay = Some(Duration::from_millis(1200));
    let stop = Arc::new(AtomicBool::new(false));
    let writer = {
        let (db, stop, job, tasks) = (repo.db().clone(), stop.clone(), job.clone(), tasks.clone());
        tokio::spawn(async move {
            // Every writer transaction attempt, committed or retried after
            // a conflict with the chain.
            let held = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            while !stop.load(Ordering::Acquire) {
                let (job, tasks) = (job.clone(), tasks.clone());
                let _ = run_tx(&db, RetryPolicy::default(), |tx| {
                    let (job, tasks, held) = (job.clone(), tasks.clone(), held.clone());
                    async move {
                        held.fetch_add(1, Ordering::AcqRel);
                        jobs::bump(&tx, "job_guard", &job).await?;
                        for t in &tasks {
                            jobs::run(
                                &tx,
                                tx.query("UPDATE $t SET updated_at = time::now() RETURN NONE")
                                    .bind(("t", jobs::rid("task", t))),
                            )
                            .await?;
                        }
                        tokio::time::sleep(Duration::from_millis(900)).await;
                        Ok(())
                    }
                })
                .await;
                tokio::time::sleep(Duration::from_millis(150)).await;
            }
            held.load(Ordering::Acquire)
        })
    };
    let cancel = Arc::new(AtomicBool::new(false));
    let config = ExecConfig {
        lease_secs: 6,
        heartbeat: Duration::from_millis(250),
    };
    let out = surreal_stages::drain(&repo, "w1", &mut mock, &cancel, &config).await;
    stop.store(true, Ordering::Release);
    let held = writer.await.unwrap();
    let out = out.unwrap_or_else(|e| panic!("drain failed: {e:#}"));
    assert!(held >= 2, "contention actually happened");
    assert_eq!(out.len(), 4, "{out:?}");
    assert!(
        out.iter()
            .all(|o| matches!(o, StageOutcome::Committed { .. })),
        "{out:?}"
    );
    assert_eq!(
        mock.requests().len(),
        4,
        "no repeated successful stage calls"
    );
    let mut r = repo
        .db()
        .query(
            "RETURN math::sum((SELECT VALUE recoveries FROM task WHERE job = $j));
             SELECT VALUE state FROM task WHERE job = $j;",
        )
        .bind(("j", jobs::rid("job", &job)))
        .await
        .unwrap()
        .check()
        .unwrap();
    let recoveries: Option<i64> = r.take(0).unwrap();
    let states: Vec<String> = r.take(1).unwrap();
    assert_eq!(recoveries.unwrap_or(0), 0);
    assert_eq!(states.len(), 4);
    assert!(states.iter().all(|s| s == "completed"), "{states:?}");
    // Each published stage merged into the displayed brief, with its leads.
    let (generation, claims, inferences) = crate::storage::brief::published_brief(&repo, "a1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(generation, 1);
    assert_eq!(claims["claims"].as_array().map(Vec::len), Some(2));
    assert_eq!(inferences["inferences"].as_array().map(Vec::len), Some(1));
    assert_eq!(
        crate::storage::brief::leads(&repo, "a1")
            .await
            .unwrap()
            .len(),
        1
    );
    // Unchanged inputs attach to the completed chain.
    let again = surreal_stages::ensure(&repo, &inputs("r1")).await.unwrap();
    assert_eq!((again.job_id, again.created), (job, false));
}
