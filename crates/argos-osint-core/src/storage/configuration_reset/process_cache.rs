//! One actual process cache clear shared by private HTTP and gateway owners.

use std::path::PathBuf;

use super::super::repo::Repository;
use super::ResetError;
use crate::config_commit;
use crate::config_commit::ConfigLock;
use crate::current_configuration;
use crate::provider::ModelAssignment;
use crate::provider::SettingsFile;

/// Validate actual fence/drain and empty current files before dropping the
/// real process AuthFile cache. The caller records its own in-memory ACK only
/// after this returns. No supplied boolean or requested epoch is an ACK.
pub async fn clear_process_cache(
    repo: &Repository,
    root: PathBuf,
    scope: &str,
    epoch: &str,
) -> Result<(), ResetError> {
    super::verify_owner_clear(repo, scope, epoch).await?;
    tokio::task::spawn_blocking(move || {
        let lock = ConfigLock::acquire_at(&root).map_err(|_| ResetError::Unavailable)?;
        let snapshot = current_configuration::read_locked(&root, &lock)
            .map_err(|_| ResetError::UnreviewedSources)?;
        let mut empty = SettingsFile::default();
        empty.defaults.recon_agent = Some(ModelAssignment::default());
        empty.defaults.recon_decisions = Some(ModelAssignment::default());
        empty.defaults.recon_analyst = Some(ModelAssignment::default());
        empty.defaults.atlas_evaluator = Some(ModelAssignment::default());
        empty.defaults.atlas_analyst = Some(ModelAssignment::default());
        empty.defaults.brain_summary = Some(ModelAssignment::default());
        let actual =
            toml::Value::try_from(&snapshot.settings).map_err(|_| ResetError::Unavailable)?;
        let expected = toml::Value::try_from(&empty).map_err(|_| ResetError::Unavailable)?;
        if actual != expected
            || !snapshot.auth.accounts.is_empty()
            || snapshot.auth.text.is_some()
            || snapshot.auth.voice.is_some()
            || !snapshot.quotas.settings.is_empty()
        {
            return Err(ResetError::UnreviewedSources);
        }
        crate::secrets::invalidate_auth_cache();
        config_commit::notify_reload();
        Ok(())
    })
    .await
    .map_err(|_| ResetError::Unavailable)?
}
