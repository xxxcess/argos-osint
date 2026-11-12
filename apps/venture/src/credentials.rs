//! Explicit purpose credentials for the external gateway, independent of the
//! retained legacy API token. Provisioning never rotates an existing authority.

use std::collections::BTreeSet;
use std::fs;
use std::fs::File;
use std::io::Read;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;

use anyhow::ensure;
use anyhow::Context;
use anyhow::Result;
use fleetworks_workspace::Workspace;
use serde::Deserialize;
use serde::Serialize;
use serde_json::json;

const LIMIT: u64 = 64 * 1024;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Gateway {
    pub scope_id: String,
    pub access_epoch: String,
    pub manifest: PathBuf,
    pub native_token: PathBuf,
    pub web_token: PathBuf,
    pub operator_token: PathBuf,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Authority {
    version: u16,
    scope_id: String,
    access_epoch: String,
}

fn private_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    let metadata = fs::symlink_metadata(path)?;
    ensure!(
        metadata.is_file() && !metadata.file_type().is_symlink() && metadata.len() <= LIMIT,
        "invalid gateway provision file"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        use std::os::unix::fs::PermissionsExt;
        ensure!(
            metadata.permissions().mode() & 0o077 == 0 && metadata.nlink() == 1,
            "gateway provision file must be private and singly linked"
        );
    }
    let file = File::open(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let opened = file.metadata()?;
        ensure!(
            (opened.dev(), opened.ino()) == (metadata.dev(), metadata.ino()),
            "gateway provision file changed while opening"
        );
    }
    let mut body = Vec::new();
    file.take(LIMIT + 1).read_to_end(&mut body)?;
    ensure!(
        body.len() as u64 <= LIMIT,
        "gateway provision file exceeds bound"
    );
    Ok(serde_json::from_slice(&body)?)
}

fn create_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let mut file = tempfile::NamedTempFile::new_in(
        path.parent().context("gateway provision parent missing")?,
    )?;
    serde_json::to_writer_pretty(&mut file, value)?;
    file.write_all(b"\n")?;
    file.as_file().sync_all()?;
    file.persist_noclobber(path).map_err(|error| error.error)?;
    File::open(path.parent().context("gateway provision parent missing")?)?.sync_all()?;
    Ok(())
}

fn opaque(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= 128 && !value.chars().any(char::is_control)
}

impl Gateway {
    pub fn validates_workspace(&self, workspace: &Path) -> bool {
        opaque(&self.scope_id)
            && opaque(&self.access_epoch)
            && self.manifest == workspace.join("secrets/gateway-credentials.json")
            && self.native_token == workspace.join("secrets/native-api-token")
            && self.web_token == workspace.join("secrets/web-relay-token")
            && self.operator_token == workspace.join("secrets/operator-api-token")
    }

    pub fn ensure_distinct(&self, workspace: &Workspace, private: &[PathBuf]) -> Result<()> {
        let mut values = BTreeSet::new();
        for path in [&self.native_token, &self.web_token, &self.operator_token]
            .into_iter()
            .chain(private)
        {
            ensure!(
                values.insert(workspace.read_api_token_file(path)?),
                "purpose and private service credentials must differ"
            );
        }
        let old = workspace.root().join("secrets/api-token");
        if old.try_exists()? {
            if let Ok(token) = workspace.read_api_token_file(&old) {
                ensure!(
                    !values.contains(&token),
                    "legacy API token cannot acquire purpose or service authority"
                );
            }
        }
        Ok(())
    }
}

/// Blocking startup provisioning, under Venture's lifecycle lock during `up`.
/// Existing files must exactly match; no legacy bearer is imported or upgraded.
pub(crate) fn provision(workspace: &Workspace) -> Result<Gateway> {
    let environment = workspace.environment()?;
    let value = |key| {
        environment
            .get(key)
            .map(|value| {
                value
                    .into_string()
                    .map_err(|_| anyhow::anyhow!("invalid gateway authority encoding"))
            })
            .transpose()
    };
    provision_with_values(
        workspace,
        value("FLEETWORKS_ACCESS_SCOPE_ID")?,
        value("FLEETWORKS_ACCESS_EPOCH")?,
    )
}

pub(crate) fn provision_with_values(
    workspace: &Workspace,
    scope: Option<String>,
    epoch: Option<String>,
) -> Result<Gateway> {
    workspace.initialize()?;
    let authority_path = workspace.root().join("config/gateway-authority.json");
    let manifest = workspace.root().join("secrets/gateway-credentials.json");
    let authority: Authority = if authority_path.try_exists()? {
        private_json(&authority_path)?
    } else {
        ensure!(
            !manifest.try_exists()?,
            "gateway authority is missing; explicit repair is required"
        );
        let (scope_id, access_epoch) = match (&scope, &epoch) {
            (Some(scope), Some(epoch)) => (scope.clone(), epoch.clone()),
            (None, None) => {
                let scope = workspace.initialize_secret_file("gateway-scope-id")?;
                let epoch = workspace.initialize_secret_file("gateway-access-epoch")?;
                (
                    format!("gateway-{}", workspace.read_api_token_file(&scope)?),
                    workspace.read_api_token_file(&epoch)?,
                )
            }
            _ => anyhow::bail!("gateway scope and epoch must be provisioned together"),
        };
        ensure!(
            opaque(&scope_id) && opaque(&access_epoch),
            "invalid explicit gateway authority"
        );
        let authority = Authority {
            version: 1,
            scope_id,
            access_epoch,
        };
        create_json(&authority_path, &authority)?;
        authority
    };
    ensure!(
        authority.version == 1 && opaque(&authority.scope_id) && opaque(&authority.access_epoch),
        "invalid retained gateway authority"
    );
    ensure!(
        scope
            .as_ref()
            .is_none_or(|scope| scope == &authority.scope_id)
            && epoch
                .as_ref()
                .is_none_or(|epoch| epoch == &authority.access_epoch),
        "gateway authority conflicts with retained provisioning"
    );
    let gateway = Gateway {
        scope_id: authority.scope_id.clone(),
        access_epoch: authority.access_epoch.clone(),
        manifest,
        native_token: workspace.initialize_secret_file("native-api-token")?,
        web_token: workspace.initialize_secret_file("web-relay-token")?,
        operator_token: workspace.initialize_secret_file("operator-api-token")?,
    };
    gateway.ensure_distinct(workspace, &[])?;
    let expected = json!({"version":1,"scope_id":authority.scope_id,"access_epoch":authority.access_epoch,"credentials":[
        {"id":"venture-native","purpose":"native","token_file":"/run/secrets/native_api_token","permissions":["read_backend","register_workstation","rebind_workstation","manage_configuration","maintain_backend","submit_interactive","read_workstation_activity","retry_job","cancel_job"]},
        {"id":"venture-web-relay","purpose":"web_relay","token_file":"/run/secrets/web_relay_token","permissions":["read_backend","resolve_web_session","submit_interactive","read_workstation_activity","retry_job","cancel_job"]},
        {"id":"venture-operator-read","purpose":"operator","token_file":"/run/secrets/operator_api_token","permissions":["read_backend"]}
    ]});
    if gateway.manifest.try_exists()? {
        let actual: serde_json::Value = private_json(&gateway.manifest)?;
        ensure!(
            actual == expected,
            "gateway purpose manifest changed; explicit repair is required"
        );
    } else {
        create_json(&gateway.manifest, &expected)?;
    }
    Ok(gateway)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provisioning_preserves_legacy_bytes_and_authority_across_restarts() {
        let temporary = tempfile::tempdir().expect("fixture");
        let workspace = Workspace::resolve(Some(temporary.path())).expect("workspace");
        workspace
            .initialize_api_token()
            .expect("retained legacy token");
        let legacy = fs::read(workspace.root().join("secrets/api-token")).expect("old bytes");
        let first = provision(&workspace).expect("purpose provisioning");
        assert_eq!(provision(&workspace).expect("restart"), first);
        assert_eq!(
            fs::read(workspace.root().join("secrets/api-token")).expect("retained"),
            legacy
        );
        let manifest: serde_json::Value = private_json(&first.manifest).expect("manifest");
        assert_eq!(
            manifest["credentials"][2]["permissions"],
            json!(["read_backend"])
        );
        assert!(manifest["credentials"]
            .as_array()
            .expect("entries")
            .iter()
            .all(|entry| entry["token_file"] != "/run/secrets/api_token"));
    }

    #[test]
    fn conflicting_or_missing_persisted_authority_never_silently_rotates() {
        let temporary = tempfile::tempdir().expect("fixture");
        let workspace = Workspace::resolve(Some(temporary.path())).expect("workspace");
        let gateway = provision(&workspace).expect("provision");
        let bytes = fs::read(&gateway.manifest).expect("manifest");
        fs::remove_file(workspace.root().join("config/gateway-authority.json"))
            .expect("simulate missing authority");
        assert!(provision(&workspace).is_err());
        assert_eq!(
            fs::read(gateway.manifest).expect("retained manifest"),
            bytes
        );
    }

    #[test]
    fn explicit_authority_is_persisted_and_conflicts_leave_files_untouched() {
        let temporary = tempfile::tempdir().expect("fixture");
        let workspace = Workspace::resolve(Some(temporary.path())).expect("workspace");
        let gateway = provision_with_values(
            &workspace,
            Some("fixture-scope".into()),
            Some("fixture-epoch".into()),
        )
        .expect("explicit authority");
        let bytes = fs::read(&gateway.manifest).expect("manifest");
        assert!(provision_with_values(
            &workspace,
            Some("other-scope".into()),
            Some("fixture-epoch".into())
        )
        .is_err());
        assert_eq!(fs::read(&gateway.manifest).expect("unchanged"), bytes);
        let changed = json!({"version":1,"credentials":[]});
        fs::write(
            &gateway.manifest,
            serde_json::to_vec(&changed).expect("fixture JSON"),
        )
        .expect("simulate changed manifest");
        assert!(provision_with_values(
            &workspace,
            Some("fixture-scope".into()),
            Some("fixture-epoch".into())
        )
        .is_err());
        assert_eq!(
            fs::read(&gateway.manifest).expect("retained changed manifest"),
            serde_json::to_vec(&changed).expect("fixture JSON")
        );
    }
}
