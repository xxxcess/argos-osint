//! Stable volume binding for protected macOS settings.

use std::ffi::CStr;
use std::io;
use std::mem;
use std::os::fd::AsRawFd as _;

use anyhow::Context as _;
use anyhow::Result;
use rustix::fd::OwnedFd;
use rustix::fs::AtFlags;
use serde::Deserialize;
use serde::Serialize;

use super::fs;
use super::fs::ObjectIdentity;
use super::state::Catalog;
use super::OpenMode;

pub(super) const FILE: &str = ".native-volume.json";
const MAX_BYTES: u64 = 1024;

#[derive(Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct VolumeBinding {
    version: u16,
    installation_marker: String,
    root_object: ObjectIdentity,
    uuid: [u8; 16],
}

/// The caller has checked the current root, inode, owner, and catalog under the native lock.
pub(super) fn verify_root(
    fd: &OwnedFd,
    catalog: &Catalog,
    live: ObjectIdentity,
    mode: OpenMode,
) -> Result<()> {
    let uuid = apfs_volume_uuid(fd)?;
    let binding = uuid.map(|uuid| VolumeBinding {
        version: 1,
        installation_marker: catalog.installation_marker.clone(),
        root_object: catalog.root_object,
        uuid,
    });
    match rustix::fs::statat(fd, FILE, AtFlags::SYMLINK_NOFOLLOW) {
        Ok(_) => {
            let (saved, _) = fs::read_json_limited::<VolumeBinding>(fd, FILE, MAX_BYTES)
                .context("read protected native volume identity")?;
            anyhow::ensure!(
                binding.as_ref() == Some(&saved),
                "native root volume identity requires repair"
            );
        }
        Err(rustix::io::Errno::NOENT) => {
            if let Some(binding) = binding {
                // Older catalogs have no UUID to compare after device renumbering.
                // Require an explicit retry before binding those to the current volume.
                anyhow::ensure!(
                    catalog.root_object == live || matches!(mode, OpenMode::RetrySettings),
                    "macOS reports a new device number for this workstation folder. Select Retry folder access to keep this installation and open its saved folders."
                );
                fs::write_json(fd, FILE, &binding, None)?;
            } else {
                anyhow::ensure!(
                    catalog.root_object == live,
                    "native root identity or format requires repair"
                );
            }
        }
        Err(error) => return Err(error).context("inspect protected native volume identity"),
    }
    Ok(())
}

fn apfs_volume_uuid(fd: &OwnedFd) -> Result<Option<[u8; 16]>> {
    let stats = rustix::fs::fstatfs(fd).context("inspect native settings filesystem")?;
    let filesystem = stats.f_fstypename.map(|byte| byte as u8);
    let local_apfs = stats.f_flags & libc::MNT_LOCAL as u32 != 0
        && CStr::from_bytes_until_nul(&filesystem).is_ok_and(|name| name == c"apfs");
    if !local_apfs {
        return Ok(None);
    }

    #[repr(C)]
    struct VolumeAttributes {
        length: u32,
        uuid: [u8; 16],
    }

    let mut attributes = libc::attrlist {
        bitmapcount: libc::ATTR_BIT_MAP_COUNT,
        reserved: 0,
        commonattr: 0,
        volattr: libc::ATTR_VOL_INFO | libc::ATTR_VOL_UUID,
        dirattr: 0,
        fileattr: 0,
        forkattr: 0,
    };
    let mut value = VolumeAttributes {
        length: 0,
        uuid: [0; 16],
    };
    let size = mem::size_of_val(&value);
    // SAFETY: The live descriptor and both structures remain valid for the call.
    // The result buffer holds the length followed by the single requested UUID.
    let result = unsafe {
        libc::fgetattrlist(
            fd.as_raw_fd(),
            std::ptr::from_mut(&mut attributes).cast(),
            std::ptr::from_mut(&mut value).cast(),
            size,
            0,
        )
    };
    if result == -1 {
        return Err(io::Error::last_os_error()).context("read APFS volume UUID");
    }
    anyhow::ensure!(
        value.length as usize == size && value.uuid != [0; 16],
        "invalid APFS volume UUID"
    );
    Ok(Some(value.uuid))
}

#[cfg(test)]
mod tests {
    use super::fs;
    use super::VolumeBinding;
    use super::FILE;
    use crate::native::NativeContainer;

    #[test]
    fn explicit_retry_rejects_a_conflicting_apfs_volume_binding() {
        let temporary = tempfile::tempdir().expect("isolated synthetic home");
        let home = temporary.path().canonicalize().expect("real home path");
        let container = NativeContainer::open_settings(&home).expect("local APFS settings");
        let (mut saved, object) = fs::read_json::<VolumeBinding>(&container.root_fd, FILE)
            .expect("persistent APFS binding");
        saved.uuid[0] ^= 1;
        fs::write_json(&container.root_fd, FILE, &saved, Some(object))
            .expect("inject a different volume UUID");
        drop(container);

        let error = NativeContainer::retry_settings_access(&home)
            .err()
            .expect("an existing volume binding cannot be replaced by a retry");
        assert_eq!(
            error.to_string(),
            "native root volume identity requires repair"
        );
    }
}
