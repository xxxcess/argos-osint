use std::os::unix::fs::PermissionsExt;

use super::*;

fn notice(kind: JobNoticeKind, transition: Option<i64>) -> JobNotice {
    JobNotice {
        key: JobNoticeKey {
            job_id: "synthetic-current-job".into(),
            job_digest: "c".repeat(64),
            kind,
            transition,
        },
        observed_at_ms: 1000,
        read: false,
    }
}

#[test]
fn job_history_survives_reopen_with_reads_and_isolates_authority() {
    let directory = tempfile::tempdir().expect("fixture home");
    let home = directory.path().canonicalize().expect("real fixture home");
    let authority = "a".repeat(64);
    let first = notice(JobNoticeKind::Started, Some(100));
    let failed = notice(JobNoticeKind::Failed, Some(200));
    let container = NativeContainer::open_settings(&home).expect("installation");
    assert!(container
        .update_job_notice_history(&authority, &[])
        .unwrap()
        .retained()
        .is_empty());
    assert!(!container
        .root()
        .join(format!(".native-inbox-jobs-{authority}.json"))
        .exists());
    container
        .update_job_notice_history(
            &authority,
            &[
                JobNoticeChange::Record(first.clone()),
                JobNoticeChange::Record(failed.clone()),
                JobNoticeChange::MarkRead(first.key.clone()),
            ],
        )
        .expect("write current notification state");
    drop(container);

    let reopened = NativeContainer::open_settings(&home).expect("reopen installation");
    let restored = reopened.update_job_notice_history(&authority, &[]).unwrap();
    assert_eq!(restored.retained().len(), 2);
    assert_eq!(
        restored.retained()[0],
        JobNotice {
            read: true,
            ..first
        }
    );
    assert_eq!(restored.retained()[1], failed);
    let other_authority = "b".repeat(64);
    assert!(reopened
        .update_job_notice_history(&other_authority, &[])
        .unwrap()
        .retained()
        .is_empty());
}

#[test]
fn deletion_is_event_scoped_and_blocks_replay_even_before_snapshot_compaction() {
    let directory = tempfile::tempdir().expect("fixture home");
    let home = directory.path().canonicalize().expect("real fixture home");
    let authority = "a".repeat(64);
    let first = notice(JobNoticeKind::Failed, Some(100));
    let second = notice(JobNoticeKind::Failed, Some(200));
    let started = notice(JobNoticeKind::Started, Some(90));
    let container = NativeContainer::open_settings(&home).expect("installation");
    container
        .update_job_notice_history(
            &authority,
            &[
                JobNoticeChange::Record(first.clone()),
                JobNoticeChange::Record(started.clone()),
            ],
        )
        .expect("write current notification state");
    let store = container
        .root()
        .join(format!(".native-inbox-jobs-{authority}.json"));
    let before_compaction = std::fs::read(&store).expect("retained snapshot");
    let after = container
        .update_job_notice_history(&authority, &[JobNoticeChange::Delete(first.key.clone())])
        .expect("delete one event");
    assert_eq!(after.retained(), [started.clone()]);
    let tombstone = container
        .root()
        .join(deletions_name(&authority, &first.key));
    assert!(!std::fs::read_to_string(&tombstone)
        .unwrap()
        .contains(&first.key.job_id));
    assert_eq!(
        std::fs::metadata(&tombstone).unwrap().permissions().mode() & 0o777,
        0o600
    );
    std::fs::write(store, before_compaction).expect("simulate interruption before compaction");
    drop(container);

    let reopened = NativeContainer::open_settings(&home).expect("reopen installation");
    let after = reopened
        .update_job_notice_history(
            &authority,
            &[
                JobNoticeChange::Record(first),
                JobNoticeChange::Record(notice(JobNoticeKind::Failed, None)),
                JobNoticeChange::Record(second.clone()),
            ],
        )
        .expect("reconcile replay and a later failure");
    assert_eq!(after.retained(), [started, second]);
}
