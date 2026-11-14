//! The optimistic configuration revision must survive a fresh process.

use std::path::PathBuf;
use std::process::Command;

use argos_osint_core::config_commit::CommitFile;
use argos_osint_core::config_commit::ConfigLock;

const ROOT_ENV: &str = "ARGOS_TEST_CONFIG_RESTART_ROOT";
const REVISION_ENV: &str = "ARGOS_TEST_CONFIG_RESTART_REVISION";
const NONCE_ENV: &str = "ARGOS_TEST_CONFIG_RESTART_NONCE";

#[test]
fn configuration_revision_survives_process_restart() {
    if let Some(root) = std::env::var_os(ROOT_ENV) {
        let root = PathBuf::from(root)
            .canonicalize()
            .expect("parent-created fixture root");
        let temporary = std::env::temp_dir()
            .canonicalize()
            .expect("temporary fixture parent");
        assert!(
            root.starts_with(temporary),
            "child only uses a temporary fixture"
        );
        let nonce = std::env::var(NONCE_ENV).expect("parent fixture nonce");
        assert_eq!(
            std::fs::read_to_string(root.join(".restart-fixture")).expect("owned fixture marker"),
            nonce
        );
        let expected: u64 = std::env::var(REVISION_ENV)
            .expect("parent committed revision")
            .parse()
            .expect("revision number");
        let lock = ConfigLock::acquire_at(&root).expect("new process lock");
        assert_eq!(
            lock.generation(),
            expected,
            "a restarted process must reject an old form revision"
        );
        let revision = lock
            .commit_files(&[CommitFile::new(
                "revision_fixture",
                root.join("config.toml"),
                "value = 'second'\n".into(),
            )])
            .expect("new process commit");
        assert!(
            revision > expected,
            "the durable revision must advance across processes"
        );
        return;
    }
    let fixture = tempfile::tempdir().expect("isolated configuration fixture");
    let root = fixture
        .path()
        .canonicalize()
        .expect("canonical temporary root");
    let nonce = format!(
        "{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("fixture clock")
            .as_nanos()
    );
    std::fs::write(root.join(".restart-fixture"), &nonce).expect("non-secret fixture marker");
    let lock = ConfigLock::acquire_at(&root).expect("first process lock");
    let first = lock
        .commit_files(&[CommitFile::new(
            "revision_fixture",
            root.join("config.toml"),
            "value = 'first'\n".into(),
        )])
        .expect("first revision");
    drop(lock);
    let output =
        Command::new(std::env::current_exe().expect("current integration test executable"))
            .args([
                "--exact",
                "configuration_revision_survives_process_restart",
                "--nocapture",
            ])
            .env(ROOT_ENV, &root)
            .env(REVISION_ENV, first.to_string())
            .env(NONCE_ENV, nonce)
            .output()
            .expect("fresh process fixture");
    assert!(
        output.status.success(),
        "fresh process failed: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let lock = ConfigLock::acquire_at(&root).expect("parent reopens after child exit");
    assert!(lock.generation() > first);
    assert_eq!(
        std::fs::read_to_string(root.join("config.toml")).expect("committed fixture"),
        "value = 'second'\n"
    );
}
