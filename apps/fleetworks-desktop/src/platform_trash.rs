//! Identity-reference macOS Trash adapter. Invoked only on a blocking worker
//! after explicit Delete confirmation and all authoritative retirement receipts.

use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;
use fleetworks_workstation::native::TrashAdapter;
use fleetworks_workstation::native::TrashReference;
use fleetworks_workstation::native::TrashTarget;
use objc2_foundation::NSFileManager;
use objc2_foundation::NSSearchPathDirectory;
use objc2_foundation::NSSearchPathDomainMask;
use objc2_foundation::NSString;
use objc2_foundation::NSURL;

/// No arbitrary destination, path-only fallback, shell command or recursion.
pub(crate) struct MacTrash;

impl TrashAdapter for MacTrash {
    fn prepare(&self, target: &TrashTarget<'_>) -> Result<TrashReference> {
        target.verify_original()?;
        let path = target
            .original_path()
            .to_str()
            .context("folder name is not valid UTF-8")?;
        let url = NSURL::fileURLWithPath_isDirectory(&NSString::from_str(path), true);
        let reference = url
            .fileReferenceURL()
            .context("macOS could not create a stable folder reference")?;
        anyhow::ensure!(
            reference.isFileReferenceURL() && reference.isFileURL(),
            "macOS did not provide a local identity reference"
        );
        let reference_text = reference
            .absoluteString()
            .context("macOS folder reference is missing")?
            .to_string();
        let reference = TrashReference::new(reference_text, PathBuf::new());
        let resolved = self
            .resolve(&reference)?
            .context("macOS folder reference cannot be resolved")?;
        target.verify_original_reference(&resolved)?;
        // This fixed OS directory may be created only in this explicitly
        // confirmed Delete path. The manager persists it before moving bytes.
        let trash = NSFileManager::defaultManager()
            .URLForDirectory_inDomain_appropriateForURL_create_error(
                NSSearchPathDirectory::TrashDirectory,
                NSSearchPathDomainMask::UserDomainMask,
                Some(url.as_ref()),
                true,
            )
            .map_err(|_| anyhow::anyhow!("macOS could not prepare this volume's user Trash"))?;
        anyhow::ensure!(
            trash.isFileURL(),
            "macOS returned a non-local Trash directory"
        );
        let path = trash
            .path()
            .context("macOS Trash directory is unavailable")?;
        let trash_directory = PathBuf::from(path.to_string());
        anyhow::ensure!(
            trash_directory.is_absolute(),
            "macOS Trash directory must be absolute"
        );
        Ok(TrashReference::new(reference.reference, trash_directory))
    }

    fn resolve(&self, reference: &TrashReference) -> Result<Option<PathBuf>> {
        let url = NSURL::URLWithString(&NSString::from_str(&reference.reference))
            .context("saved macOS folder reference is invalid")?;
        anyhow::ensure!(
            url.isFileReferenceURL() && url.isFileURL(),
            "saved reference must be a local file identity"
        );
        let Some(path) = url.filePathURL().and_then(|url| url.path()) else {
            return Ok(None);
        };
        let path = PathBuf::from(path.to_string());
        anyhow::ensure!(
            path.is_absolute(),
            "resolved macOS folder location must be absolute"
        );
        Ok(Some(path))
    }

    fn move_to_trash(&self, target: &TrashTarget<'_>, reference: &TrashReference) -> Result<()> {
        let resolved = self
            .resolve(reference)?
            .context("saved folder reference is unavailable; cleanup stays pending")?;
        target.verify_original_reference(&resolved)?;
        let url = NSURL::URLWithString(&NSString::from_str(&reference.reference))
            .context("saved macOS folder reference is invalid")?;
        anyhow::ensure!(
            url.isFileReferenceURL() && url.isFileURL(),
            "Trash requires the saved local identity reference"
        );
        NSFileManager::defaultManager()
            .trashItemAtURL_resultingItemURL_error(&url, None)
            .map_err(|_| anyhow::anyhow!("macOS did not confirm the Trash move; recovery will recheck the saved folder reference"))
    }
}
