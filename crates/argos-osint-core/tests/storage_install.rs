//! `argos storage install` archive checks (migration phase 4). Hermetic:
//! uses a stand-in archive; the real pinned hash is checked by
//! `host_pin_matches_design` only.
#![cfg(all(feature = "surreal", unix))]

use std::process::Command;

use argos_osint_core::storage::home::Layout;
use argos_osint_core::storage::install::{self, install_archive, sha256_file};

fn fake_archive(dir: &std::path::Path, version: &str) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let src = dir.join("src");
    std::fs::create_dir_all(&src).unwrap();
    let bin = src.join("surreal");
    std::fs::write(&bin, format!("#!/bin/sh\necho '{version} for test'\n")).unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    let archive = dir.join(format!("surreal-{version}.tgz"));
    assert!(Command::new("tar")
        .arg("-czf")
        .arg(&archive)
        .arg("-C")
        .arg(&src)
        .arg("surreal")
        .status()
        .unwrap()
        .success());
    archive
}

#[test]
fn install_checks_hash_and_version_then_installs_atomically() {
    let dir = tempfile::tempdir().unwrap();
    let layout = Layout::new(dir.path().join("home"));
    let good = fake_archive(dir.path(), "3.3.4");
    let sha = sha256_file(&good).unwrap();

    let err = install_archive(&layout, &good, &"0".repeat(64))
        .unwrap_err()
        .to_string();
    assert!(err.contains("not installing"), "{err}");
    assert!(!layout.server_bin().exists());

    let wrong_dir = dir.path().join("wrong");
    std::fs::create_dir_all(&wrong_dir).unwrap();
    let wrong = fake_archive(&wrong_dir, "3.3.3");
    let err = install_archive(&layout, &wrong, &sha256_file(&wrong).unwrap())
        .unwrap_err()
        .to_string();
    assert!(err.contains("expected 3.3.4"), "{err}");
    assert!(!layout.server_bin().exists());

    let path = install_archive(&layout, &good, &sha).unwrap();
    assert_eq!(path, layout.server_bin());
    let out = Command::new(&path).arg("version").output().unwrap();
    assert!(String::from_utf8_lossy(&out.stdout).starts_with("3.3.4"));
    // Only the binary is left in bin/ (no temp dirs).
    let names: Vec<_> = std::fs::read_dir(layout.home.join("bin"))
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names, vec![std::ffi::OsString::from("surreal-3.3.4")]);
}

#[test]
fn host_pin_matches_design() {
    let (asset, sha) = install::host_pin().unwrap();
    assert_eq!(sha.len(), 64);
    assert_eq!(
        install::download_url(asset),
        format!("https://github.com/surrealdb/surrealdb/releases/download/v3.3.4/surreal-v3.3.4.{asset}.tgz")
    );
    if asset == "linux-amd64" {
        assert_eq!(
            sha,
            "6593071833ad9c85cf533cf70b1643eb08966c868f8624001e30839ea29a2689"
        );
    }
}
