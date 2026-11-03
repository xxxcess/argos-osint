//! Select the repo's Node environment through nvm before running JS tools.
use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use tokio::process::Command;

#[derive(Deserialize)]
pub(super) struct NodeEnvironment {
    path: String,
    bin: PathBuf,
    dir: PathBuf,
    version: String,
}

fn valid_pin(pin: &str) -> bool {
    pin.split('.').count() == 3
        && pin
            .split('.')
            .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
        && pin.starts_with("24.")
}

fn nvm_script(dir: &Path) -> Result<PathBuf> {
    [
        dir.join("nvm.sh"),
        PathBuf::from("/opt/homebrew/opt/nvm/nvm.sh"),
        PathBuf::from("/usr/local/opt/nvm/nvm.sh"),
    ]
    .into_iter()
    .find(|path| path.is_file())
    .context(
        "nvm.sh not found; install nvm or set NVM_DIR to its installation before launching Web",
    )
}

impl NodeEnvironment {
    pub async fn select(root: &Path) -> Result<Self> {
        let home = std::env::var_os("HOME").context("HOME is required to locate nvm")?;
        let dir = std::env::var_os("NVM_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(home).join(".nvm"));
        Self::select_from(root, &dir).await
    }

    async fn select_from(root: &Path, dir: &Path) -> Result<Self> {
        let pin = std::fs::read_to_string(root.join(".nvmrc")).context("read project .nvmrc")?;
        let pin = pin.trim();
        anyhow::ensure!(valid_pin(pin), ".nvmrc must pin an exact Node 24 version");
        let script = nvm_script(dir)?;
        // Positional arguments keep filesystem paths and the pin out of shell
        // source. Only the selected Node process emits the environment subset.
        let mut command = Command::new("bash");
        command.current_dir(root).env("NVM_DIR", dir).kill_on_drop(true).args([
            "-c",
            r#"set -e
. "$1" --no-use
nvm use --silent "$2" >/dev/null
node -p 'JSON.stringify({path:process.env.PATH,bin:process.env.NVM_BIN,dir:process.env.NVM_DIR,version:process.version})'"#,
            "argos-nvm",
        ]).arg(script).arg(pin);
        let output = tokio::time::timeout(std::time::Duration::from_secs(30), command.output())
            .await
            .context("nvm use timed out")??;
        anyhow::ensure!(
            output.status.success(),
            "nvm use {pin} failed; run `nvm install {pin}` first: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
        let stdout = String::from_utf8(output.stdout)?;
        let selected: Self = serde_json::from_str(
            stdout
                .lines()
                .last()
                .context("nvm did not return a Node environment")?,
        )
        .context("read selected nvm environment")?;
        anyhow::ensure!(
            selected.version == format!("v{pin}"),
            "nvm selected {}, expected v{pin}",
            selected.version
        );
        println!("Node {} selected with nvm (.nvmrc)", selected.version);
        Ok(selected)
    }

    pub fn command(&self, tool: &str) -> Command {
        let mut command = if tool == "node" {
            Command::new(self.bin.join("node"))
        } else {
            Command::new(tool)
        };
        command
            .kill_on_drop(true)
            .env("PATH", &self.path)
            .env("NVM_BIN", &self.bin)
            .env("NVM_DIR", &self.dir);
        command
    }

    pub async fn version(&self, tool: &str) -> Result<String> {
        let output = self
            .command(tool)
            .arg("--version")
            .output()
            .await
            .with_context(|| format!("{tool} unavailable in the nvm Node environment"))?;
        anyhow::ensure!(
            output.status.success(),
            "{tool} --version failed in the nvm Node environment"
        );
        Ok(String::from_utf8_lossy(&output.stdout).trim().into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn target_is_an_exact_node_24_pin() {
        assert!(valid_pin("24.22.1"));
        for pin in ["25.8.0", "lts/*", "24", "24.1.", "24.1.0;exit", "24.1.0\n"] {
            assert!(!valid_pin(pin));
        }
    }
    #[test]
    fn custom_nvm_directory_takes_precedence() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("nvm.sh");
        std::fs::write(&script, "").unwrap();
        assert_eq!(nvm_script(dir.path()).unwrap(), script);
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn nvm_use_selects_the_environment_for_child_commands() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".nvmrc"), "24.22.1\n").unwrap();
        let bin = dir.path().join("selected/bin");
        std::fs::create_dir_all(&bin).unwrap();
        let path = format!("{}:/usr/bin:/bin", bin.display());
        let json = serde_json::json!({"path":path,"bin":bin,"dir":dir.path(),"version":"v24.22.1"});
        let executable = bin.join("node");
        std::fs::write(
            &executable,
            format!("#!/bin/sh\ncat <<'ARGOS_TEST_JSON'\n{json}\nARGOS_TEST_JSON\n"),
        )
        .unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::write(
            dir.path().join("nvm.sh"),
            r#"
nvm() {
    [ "$1" = use ] && [ "$2" = --silent ] && [ "$3" = 24.22.1 ] || return 99
    printf '%s' "$3" > "$NVM_DIR/used"
    export NVM_BIN="$NVM_DIR/selected/bin"
    export PATH="$NVM_BIN:/usr/bin:/bin"
}
"#,
        )
        .unwrap();
        let environment = NodeEnvironment::select_from(dir.path(), dir.path())
            .await
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.path().join("used")).unwrap(),
            "24.22.1"
        );
        let command = environment.command("node");
        assert_eq!(command.as_std().get_program(), executable.as_os_str());
        assert!(command
            .as_std()
            .get_envs()
            .any(|(key, value)| key == "PATH" && value == Some(std::ffi::OsStr::new(&path))));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn node_command_configures_kill_on_drop() {
        let env = NodeEnvironment {
            path: "/usr/bin:/bin".into(),
            bin: "/bin".into(),
            dir: "/tmp".into(),
            version: "v24.22.1".into(),
        };
        let mut cmd = env.command("sleep");
        cmd.arg("30");
        let child = cmd.spawn().unwrap();
        let pid = child.id().unwrap();
        assert_eq!(unsafe { libc::kill(pid as i32, 0) }, 0);
        drop(child);
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert_eq!(unsafe { libc::kill(pid as i32, 0) }, -1);
    }
}
