//! OS-enforced isolation for the private Embedded model child.

use std::fs;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

use anyhow::ensure;
use anyhow::Result;
use tokio::process::Command;

/// State credentials and datastore files must be outside every readable directory.
pub(super) fn protect_state(state: &Path, pack: &Path) -> Result<()> {
    let state = state.canonicalize()?;
    for readable in std::iter::once(pack).chain(LOADER_DIRS.iter().map(Path::new)) {
        let directory = match fs::metadata(readable) {
            Ok(directory) => directory,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        for ancestor in state.ancestors() {
            let ancestor = fs::metadata(ancestor)?;
            ensure!(
                (ancestor.dev(), ancestor.ino()) != (directory.dev(), directory.ino()),
                "QA pack isolation cannot share a readable directory with Embedded state"
            );
        }
    }
    Ok(())
}

#[cfg(target_os = "macos")]
const LOADER_DIRS: &[&str] = &[
    "/usr/lib",
    "/System/Library",
    "/System/Cryptexes/OS/usr/lib",
    "/System/Volumes/Preboot/Cryptexes/OS/usr/lib",
];

#[cfg(target_os = "macos")]
pub(super) fn command(pack: &Path, worker: &Path) -> Result<Command> {
    use std::ffi::OsString;

    const PROFILE: &str = r#"
        (version 1)
        (deny default)
        (allow process-exec (literal (param "worker")))
        (allow file-read* (subpath (param "pack")))
        (allow file-read-data (literal "/"))
        (allow file-read*
            (subpath "/usr/lib")
            (subpath "/System/Library")
            (subpath "/System/Cryptexes/OS/usr/lib")
            (subpath "/System/Volumes/Preboot/Cryptexes/OS/usr/lib")
            (literal "/dev/null") (literal "/dev/random") (literal "/dev/urandom"))
        (allow sysctl-read)
    "#;
    let parameter = |key: &str, path: &Path| {
        let mut value = OsString::from(key);
        value.push(path.as_os_str());
        value
    };
    // The Darwin pack uses @loader_path from signed load commands. No DYLD override is needed.
    let mut command = Command::new("/usr/bin/sandbox-exec");
    command
        .env_clear()
        .arg("-p")
        .arg(PROFILE)
        .arg("-D")
        .arg(parameter("pack=", pack))
        .arg("-D")
        .arg(parameter("worker=", worker))
        .arg(worker);
    Ok(command)
}

#[cfg(target_os = "linux")]
const LOADER_DIRS: &[&str] = &["/lib", "/lib64", "/usr/lib", "/usr/lib64"];

#[cfg(target_os = "linux")]
pub(super) use linux::command;

#[cfg(target_os = "linux")]
mod linux {
    use std::fs::OpenOptions;
    use std::mem::size_of;
    use std::os::fd::AsRawFd;
    use std::os::fd::FromRawFd;
    use std::os::fd::OwnedFd;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::OpenOptionsExt;

    use super::*;

    // The Landlock ABI encodes the first 15 filesystem rights through TRUNCATE.
    const LANDLOCK_VERSION: libc::c_uint = 1;
    const MIN_LANDLOCK_ABI: libc::c_long = 3;
    const FS_READ_ONLY: u64 = (1 << 0) | (1 << 2) | (1 << 3);
    const FS_READ_FILE: u64 = 1 << 2;
    const FS_ABI_3: u64 = (1 << 15) - 1;
    const FS_IOCTL_DEV: u64 = 1 << 15;
    const LANDLOCK_RULE_PATH_BENEATH: libc::c_uint = 1;

    #[repr(C, packed)]
    struct PathRule {
        allowed_access: u64,
        parent_fd: libc::c_int,
    }

    fn allow_path(ruleset: &OwnedFd, path: &Path, rights: u64) -> io::Result<()> {
        let path = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_PATH)
            .open(path)?;
        let rule = PathRule {
            allowed_access: rights,
            parent_fd: path.as_raw_fd(),
        };
        // SAFETY: Both descriptors and the packed UAPI data remain valid for the call.
        if unsafe {
            libc::syscall(
                libc::SYS_landlock_add_rule,
                ruleset.as_raw_fd(),
                LANDLOCK_RULE_PATH_BENEATH,
                &rule as *const PathRule,
                0,
            )
        } != 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    fn rules_for(pack: &Path) -> Result<OwnedFd> {
        // SAFETY: The null descriptor data and zero size request the supported ABI version.
        let abi = unsafe {
            libc::syscall(
                libc::SYS_landlock_create_ruleset,
                std::ptr::null::<u64>(),
                0_usize,
                LANDLOCK_VERSION,
            )
        };
        if abi < MIN_LANDLOCK_ABI {
            let os_error = (abi < 0).then(io::Error::last_os_error);
            tracing::warn!(
                qa_warm_stage = "sandbox_setup",
                qa_warm_reason = "landlock_abi",
                landlock_abi = abi,
                sanitized_os_error = ?os_error.as_ref().and_then(io::Error::raw_os_error),
                "Inbox QA requires Linux Landlock ABI 3 or newer"
            );
            return Err(anyhow::anyhow!(
                "Inbox QA requires Linux Landlock ABI 3 or newer"
            ));
        }
        let handled_access_fs = FS_ABI_3 | if abi >= 5 { FS_IOCTL_DEV } else { 0 };
        // SAFETY: The first field of landlock_ruleset_attr is the initialized u64 filesystem mask.
        let raw = unsafe {
            libc::syscall(
                libc::SYS_landlock_create_ruleset,
                &handled_access_fs as *const u64,
                size_of::<u64>(),
                0,
            )
        };
        if raw < 0 {
            return Err(io::Error::last_os_error().into());
        }
        // SAFETY: A successful create_ruleset gives this caller a new close-on-exec descriptor.
        let ruleset = unsafe { OwnedFd::from_raw_fd(i32::try_from(raw)?) };
        allow_path(&ruleset, pack, FS_READ_ONLY)?;
        for path in LOADER_DIRS {
            allow_if_present(&ruleset, path, FS_READ_ONLY)?;
        }
        for path in [
            "/etc/ld.so.cache",
            "/dev/null",
            "/dev/urandom",
            "/dev/random",
        ] {
            allow_if_present(&ruleset, path, FS_READ_FILE)?;
        }
        Ok(ruleset)
    }

    fn allow_if_present(ruleset: &OwnedFd, path: &str, rights: u64) -> io::Result<()> {
        match allow_path(ruleset, Path::new(path), rights) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            result => result,
        }
    }

    fn instruction(code: u16, jt: u8, jf: u8, k: u32) -> libc::sock_filter {
        libc::sock_filter { code, jt, jf, k }
    }

    fn syscall_filter() -> Vec<libc::sock_filter> {
        const BPF_LOAD_WORD_ABSOLUTE: u16 = 0x20;
        const BPF_JUMP_EQUAL: u16 = 0x15;
        const BPF_JUMP_SET: u16 = 0x45;
        const BPF_RETURN: u16 = 0x06;
        const SECCOMP_KILL: u32 = 0x8000_0000;
        const SECCOMP_EPERM: u32 = 0x0005_0000 | libc::EPERM as u32;
        const SECCOMP_ALLOW: u32 = 0x7fff_0000;
        #[cfg(target_arch = "x86_64")]
        const ARCH: u32 = 0xc000_003e;
        #[cfg(target_arch = "aarch64")]
        const ARCH: u32 = 0xc000_00b7;

        // Check seccomp_data.arch (offset 4), then seccomp_data.nr (offset 0).
        let mut filter = vec![
            instruction(BPF_LOAD_WORD_ABSOLUTE, 0, 0, 4),
            instruction(BPF_JUMP_EQUAL, 1, 0, ARCH),
            instruction(BPF_RETURN, 0, 0, SECCOMP_KILL),
            instruction(BPF_LOAD_WORD_ABSOLUTE, 0, 0, 0),
        ];
        if cfg!(target_arch = "x86_64") {
            // x32 uses the x86_64 audit architecture but separate syscall numbers.
            filter.push(instruction(BPF_JUMP_SET, 0, 1, 1 << 30));
            filter.push(instruction(BPF_RETURN, 0, 0, SECCOMP_KILL));
        }
        for syscall in [
            libc::SYS_socket,
            libc::SYS_socketpair,
            libc::SYS_connect,
            libc::SYS_bind,
            libc::SYS_listen,
            libc::SYS_accept,
            libc::SYS_accept4,
            libc::SYS_sendto,
            libc::SYS_recvfrom,
            libc::SYS_sendmsg,
            libc::SYS_recvmsg,
            libc::SYS_sendmmsg,
            libc::SYS_recvmmsg,
            libc::SYS_io_uring_setup,
            libc::SYS_ptrace,
            libc::SYS_process_vm_readv,
            libc::SYS_process_vm_writev,
            libc::SYS_pidfd_getfd,
            libc::SYS_open_by_handle_at,
            libc::SYS_bpf,
        ] {
            filter.push(instruction(BPF_JUMP_EQUAL, 0, 1, syscall as u32));
            filter.push(instruction(BPF_RETURN, 0, 0, SECCOMP_EPERM));
        }
        filter.push(instruction(BPF_RETURN, 0, 0, SECCOMP_ALLOW));
        filter
    }

    pub(in crate::qa_worker) fn command(pack: &Path, worker: &Path) -> Result<Command> {
        let libraries = pack.join("lib");
        ensure!(
            !libraries.as_os_str().as_bytes().contains(&b':'),
            "QA pack directory cannot contain a dynamic loader path separator"
        );
        let ruleset = rules_for(pack)?;
        let filter = syscall_filter();
        let filter_len = u16::try_from(filter.len())?;
        let mut command = Command::new(worker);
        command.env_clear().env("LD_LIBRARY_PATH", libraries);
        // SAFETY: The hook uses only initialized local data, inherited descriptors and syscalls.
        // It performs no allocation or locking between fork and exec.
        unsafe {
            command.pre_exec(move || {
                const CLOSE_RANGE_CLOEXEC: libc::c_uint = 1 << 2;
                const SECCOMP_MODE_FILTER: libc::c_ulong = 2;
                let program = libc::sock_fprog {
                    len: filter_len,
                    filter: filter.as_ptr().cast_mut(),
                };
                if libc::syscall(libc::SYS_close_range, 3_u32, u32::MAX, CLOSE_RANGE_CLOEXEC) != 0
                    || libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0
                    || libc::syscall(libc::SYS_landlock_restrict_self, ruleset.as_raw_fd(), 0) != 0
                    || libc::prctl(libc::PR_SET_SECCOMP, SECCOMP_MODE_FILTER, &program) != 0
                {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
        Ok(command)
    }
}
