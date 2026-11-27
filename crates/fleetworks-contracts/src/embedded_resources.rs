//! Local preferences and work budgets shared by native Settings and Embedded.

use serde::Deserialize;
use serde::Serialize;

use crate::actor_index::ActorIndexLimits;

pub const FILE_NAME: &str = "resource-preferences.json";
pub const MAX_PREFERENCES_BYTES: u64 = 1024;

const MIB: u64 = 1024 * 1024;
const DISK_SAFETY_RESERVE: u64 = 256 * MIB;
const MIN_INDEX_WRITER_MEMORY: u64 = 32 * MIB;

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub enum ResourcePreset {
    #[default]
    #[serde(rename = "40")]
    Forty,
    #[serde(rename = "60")]
    Sixty,
    #[serde(rename = "80")]
    Eighty,
}

impl ResourcePreset {
    pub const ALL: [Self; 3] = [Self::Forty, Self::Sixty, Self::Eighty];

    pub const fn percent(self) -> u8 {
        match self {
            Self::Forty => 40,
            Self::Sixty => 60,
            Self::Eighty => 80,
        }
    }

    fn share(self, capacity: u64) -> u64 {
        (u128::from(capacity) * u128::from(self.percent()) / 100) as u64
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EmbeddedResourcePreferences {
    pub cpu: ResourcePreset,
    pub memory: ResourcePreset,
    pub disk: ResourcePreset,
}

/// Read a bounded preference file from the caller's validated private state root.
#[cfg(unix)]
pub fn read_preferences(
    state_dir: &std::path::Path,
) -> std::io::Result<EmbeddedResourcePreferences> {
    use std::fs::OpenOptions;
    use std::io::Error;
    use std::io::ErrorKind;
    use std::io::Read;
    use std::os::unix::fs::MetadataExt;
    use std::os::unix::fs::OpenOptionsExt;

    let file = match OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(state_dir.join(FILE_NAME))
    {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            return Ok(EmbeddedResourcePreferences::default());
        }
        Err(error) => return Err(error),
    };
    let metadata = file.metadata()?;
    // SAFETY: geteuid takes no arguments and returns the current effective user ID.
    let owner = unsafe { libc::geteuid() };
    if !metadata.is_file() || metadata.uid() != owner || metadata.mode() & 0o077 != 0 {
        return Err(Error::new(
            ErrorKind::PermissionDenied,
            "resource preferences must be a private regular file",
        ));
    }
    let mut bytes = Vec::new();
    file.take(MAX_PREFERENCES_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_PREFERENCES_BYTES {
        return Err(Error::new(
            ErrorKind::InvalidData,
            "resource preferences too large",
        ));
    }
    serde_json::from_slice(&bytes).map_err(|error| Error::new(ErrorKind::InvalidData, error))
}

/// One allocation for each start of the companion, measured on the host that runs it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResourceAllocation {
    pub cpu_threads: usize,
    /// Total companion RSS high watermark, including the startup footprint.
    pub memory_budget_bytes: u64,
    /// Extra bytes above the startup footprint, shared by bulk work.
    pub memory_headroom_bytes: u64,
    pub disk_budget_bytes: u64,
    /// Keep at least this many bytes free on the Embedded state volume.
    pub disk_reserve_bytes: u64,
}

impl ResourceAllocation {
    pub fn from_available(
        preferences: EmbeddedResourcePreferences,
        logical_cpus: usize,
        resident_bytes: u64,
        available_memory_bytes: u64,
        available_disk_bytes: u64,
    ) -> Self {
        let cpu_threads = (preferences.cpu.share(logical_cpus as u64) as usize).max(1);
        let memory_headroom_bytes = preferences.memory.share(available_memory_bytes);
        let disk_budget_bytes = preferences
            .disk
            .share(available_disk_bytes)
            .min(available_disk_bytes.saturating_sub(DISK_SAFETY_RESERVE));
        Self {
            cpu_threads,
            memory_budget_bytes: resident_bytes.saturating_add(memory_headroom_bytes),
            memory_headroom_bytes,
            disk_budget_bytes,
            disk_reserve_bytes: available_disk_bytes
                .saturating_sub(disk_budget_bytes)
                .max(DISK_SAFETY_RESERVE),
        }
    }

    /// Adapt the existing actor index limits without changing its ordered commits,
    /// merge policy or bounds on the number of in-flight batches.
    pub fn actor_index_limits(self) -> ActorIndexLimits {
        // Use part of the shared headroom for each writer. Opening an existing
        // index still needs the minimum configuration when writes are paused.
        let writer_memory = self.memory_headroom_bytes / 4;
        let writer_threads =
            if self.cpu_threads >= 2 && writer_memory >= 2 * MIN_INDEX_WRITER_MEMORY {
                2
            } else {
                1
            };
        let writer_memory_bytes = writer_memory.clamp(
            MIN_INDEX_WRITER_MEMORY * u64::from(writer_threads),
            256 * MIB,
        );
        let memory_pause_bytes = self.memory_budget_bytes.max(writer_memory_bytes + 1);
        let memory_resume_bytes = memory_pause_bytes
            .saturating_sub((self.memory_headroom_bytes / 8).clamp(1, 64 * MIB))
            .max(writer_memory_bytes);
        let disk_resume_bytes = self
            .disk_reserve_bytes
            .saturating_add((self.disk_budget_bytes / 8).clamp(1, 256 * MIB));
        ActorIndexLimits {
            writer_threads,
            writer_memory_bytes,
            memory_pause_bytes,
            memory_resume_bytes,
            disk_pause_free_bytes: self.disk_reserve_bytes,
            disk_resume_free_bytes: disk_resume_bytes,
            ..ActorIndexLimits::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn independent_presets_scale_available_resources() {
        let preferences: EmbeddedResourcePreferences =
            serde_json::from_str(r#"{"cpu":"80","memory":"60","disk":"40"}"#).unwrap();
        let allocation =
            ResourceAllocation::from_available(preferences, 10, 320 * MIB, 1000 * MIB, 2000 * MIB);
        assert_eq!(allocation.cpu_threads, 8);
        assert_eq!(allocation.memory_budget_bytes, 920 * MIB);
        assert_eq!(allocation.disk_budget_bytes, 800 * MIB);
        assert_eq!(allocation.disk_reserve_bytes, 1200 * MIB);
        let limits = allocation.actor_index_limits();
        assert_eq!(limits.writer_threads, 2);
        assert_eq!(limits.writer_memory_bytes, 150 * MIB);
        assert!(limits.validate().is_ok());
        assert!(serde_json::from_str::<ResourcePreset>(r#""100""#).is_err());
    }

    #[test]
    fn small_hosts_keep_a_writer_and_the_disk_safety_reserve() {
        let allocation = ResourceAllocation::from_available(
            EmbeddedResourcePreferences::default(),
            1,
            0,
            0,
            200 * MIB,
        );
        assert_eq!(allocation.cpu_threads, 1);
        assert_eq!(allocation.disk_budget_bytes, 0);
        assert_eq!(allocation.disk_reserve_bytes, DISK_SAFETY_RESERVE);
        let limits = allocation.actor_index_limits();
        assert_eq!(limits.writer_threads, 1);
        assert!(limits.validate().is_ok());
    }
}
