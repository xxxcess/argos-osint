//! Checks the vendored protoc that the lance-* build scripts use through
//! `tools/protoc` (see `.cargo/config.toml`). Depending on `protoc-bin-vendored`
//! also makes Cargo download the prebuilt protoc for this host before any build
//! script runs.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=PROTOC");
    match protoc_bin_vendored::protoc_bin_path() {
        Ok(path) => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if let Ok(meta) = std::fs::metadata(&path) {
                    if meta.permissions().mode() & 0o111 == 0 {
                        let mut perms = meta.permissions();
                        perms.set_mode(0o755);
                        let _ = std::fs::set_permissions(&path, perms);
                    }
                }
            }
        }
        Err(err) => {
            if std::env::var_os("PROTOC").is_none() {
                println!(
                    "cargo:warning=no vendored protoc for this host ({err}); install protoc (brew install protobuf) or set PROTOC"
                );
            }
        }
    }
}
