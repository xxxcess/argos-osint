//! Private Unix socket listener. LibTorch is linked only by the child executable.

#[path = "../frame.rs"]
mod frame;
#[allow(dead_code)]
#[path = "../../../../crates/fleetworks-contracts/src/qa_worker.rs"]
mod protocol;

use std::env;
use std::fs;
use std::fs::File;
use std::fs::OpenOptions;
use std::io;
use std::os::fd::AsRawFd;
use std::os::fd::OwnedFd;
use std::os::unix::fs::FileTypeExt;
use std::os::unix::fs::MetadataExt;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixListener;
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::path::PathBuf;
use std::process::Child;
use std::process::Command;
use std::process::ExitCode;
use std::process::Stdio;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::time::Duration;

use anyhow::anyhow;
use anyhow::ensure;
use anyhow::Result;

use protocol::QaWorkerCommand;
use protocol::QaWorkerReply;

const INITIAL_FRAME_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_HANDSHAKE_BYTES: usize = 128;
const IDLE_TIMEOUT: Duration = Duration::from_secs(30);
const POLL_MILLIS: libc::c_int = 100;

static STOP: AtomicBool = AtomicBool::new(false);

extern "C" fn then_stop(_: libc::c_int) {
    STOP.store(true, Ordering::Relaxed);
}

fn install_shutdown_handlers() -> io::Result<()> {
    for signal in [libc::SIGINT, libc::SIGTERM] {
        // SAFETY: The handler only performs a lock-free atomic write and outlives the process.
        let installed =
            unsafe { libc::signal(signal, then_stop as *const () as libc::sighandler_t) };
        if installed == libc::SIG_ERR {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

struct SocketPath<'a>(&'a Path);

impl Drop for SocketPath<'_> {
    fn drop(&mut self) {
        let _ = fs::remove_file(self.0);
    }
}

fn lock_directory(directory: &Path) -> Result<File> {
    let metadata = fs::symlink_metadata(directory)?;
    // SAFETY: geteuid takes no pointers and returns the identity of this process.
    let uid = unsafe { libc::geteuid() };
    ensure!(
        metadata.is_dir() && metadata.uid() == uid && metadata.mode() & 0o777 == 0o700,
        "QA socket directory must be private"
    );
    let lock = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(directory.join(".qa-listener.lock"))?;
    let metadata = lock.metadata()?;
    ensure!(
        metadata.is_file() && metadata.uid() == uid && metadata.mode() & 0o777 == 0o600,
        "QA listener lock must be private"
    );
    // SAFETY: The open File owns this fd. The lock is released only after the listener closes.
    ensure!(
        unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0,
        "QA socket already has an owner"
    );
    Ok(lock)
}

fn claim_socket(path: &Path) -> Result<UnixListener> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            // The directory lock serializes replacement with the current listener.
            ensure!(
                metadata.file_type().is_socket(),
                "unexpected file at QA socket"
            );
            fs::remove_file(path)?;
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let listener = UnixListener::bind(path)?;
    if let Err(error) = fs::set_permissions(path, fs::Permissions::from_mode(0o600)) {
        let _ = fs::remove_file(path);
        return Err(error.into());
    }
    Ok(listener)
}

fn poll_fd(fd: libc::c_int, events: libc::c_short) -> io::Result<libc::c_short> {
    let mut descriptor = libc::pollfd {
        fd,
        events,
        revents: 0,
    };
    // SAFETY: poll sees one initialized writable pollfd for the duration of the call.
    if unsafe { libc::poll(&mut descriptor, 1, POLL_MILLIS) } < 0 {
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::Interrupted {
            return Err(error);
        }
    }
    Ok(descriptor.revents)
}

struct ModelChild(Child);

impl Drop for ModelChild {
    fn drop(&mut self) {
        if !matches!(self.0.try_wait(), Ok(Some(_))) {
            if let Ok(pid) = i32::try_from(self.0.id()) {
                // SAFETY: An unreaped child owns its PID and was spawned as process group leader.
                unsafe { libc::kill(-pid, libc::SIGKILL) };
            }
            let _ = self.0.kill();
        }
        let _ = self.0.wait();
    }
}

fn serve(mut stream: UnixStream, worker: &Path, model: &Path, libraries: &Path) -> Result<()> {
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(INITIAL_FRAME_TIMEOUT))?;
    stream.set_write_timeout(Some(INITIAL_FRAME_TIMEOUT))?;
    match frame::read::<QaWorkerCommand>(&mut stream, MAX_HANDSHAKE_BYTES)? {
        Some(QaWorkerCommand::Shutdown) => {
            // An idle socket proves the previous child was already reaped.
            frame::write(&mut stream, &QaWorkerReply::Stopped)?;
            return Ok(());
        }
        Some(QaWorkerCommand::Open) => {}
        _ => return Err(anyhow!("invalid QA executor handshake")),
    }

    // Inactivity ends the child session. A valid executor renews it with bounded Ping frames
    // while its own enablement lease is current, even when there is no question to answer.
    stream.set_read_timeout(Some(IDLE_TIMEOUT))?;
    let input = OwnedFd::from(stream.try_clone()?);
    let output = OwnedFd::from(stream.try_clone()?);
    let library_variable = if cfg!(target_os = "macos") {
        "DYLD_LIBRARY_PATH"
    } else {
        "LD_LIBRARY_PATH"
    };
    let mut child = ModelChild(
        Command::new(worker)
            .arg("--model-dir")
            .arg(model)
            .current_dir(model)
            .env_clear()
            .env(library_variable, libraries)
            .env("OMP_NUM_THREADS", "1")
            .env("MKL_NUM_THREADS", "1")
            .stdin(Stdio::from(input))
            .stdout(Stdio::from(output))
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()?,
    );
    while !STOP.load(Ordering::Relaxed) {
        if child.0.try_wait()?.is_some() {
            // Only the listener confirms after it has collected the child's status.
            frame::write(&mut stream, &QaWorkerReply::Stopped)?;
            return Ok(());
        }
        let observed = poll_fd(stream.as_raw_fd(), 0)?;
        if observed & (libc::POLLHUP | libc::POLLERR | libc::POLLNVAL) != 0 {
            break;
        }
    }
    // Off/disconnect may happen during CPU work. Reap before admitting another executor.
    Ok(())
}

fn option(args: &mut impl Iterator<Item = std::ffi::OsString>, key: &str) -> Result<PathBuf> {
    ensure!(
        args.next().as_deref() == Some(key.as_ref()),
        "invalid QA listener arguments"
    );
    let path = args
        .next()
        .map(PathBuf::from)
        .ok_or_else(|| anyhow!("missing QA listener path"))?;
    ensure!(path.is_absolute(), "QA listener paths must be absolute");
    Ok(path)
}

fn run() -> Result<()> {
    let mut args = env::args_os().skip(1);
    let socket = option(&mut args, "--socket")?;
    let worker = option(&mut args, "--worker")?;
    let model = option(&mut args, "--model-dir")?;
    ensure!(args.next().is_none(), "invalid QA listener arguments");
    let worker_parent = worker
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| anyhow!("missing QA pack root"))?;
    let libraries = worker_parent.join("lib");
    ensure!(
        fs::symlink_metadata(&worker)?.is_file()
            && fs::symlink_metadata(&model)?.is_dir()
            && fs::symlink_metadata(&libraries)?.is_dir(),
        "missing immutable QA image resources"
    );
    let _lock = lock_directory(
        socket
            .parent()
            .ok_or_else(|| anyhow!("missing QA socket directory"))?,
    )?;
    install_shutdown_handlers()?;
    let listener = claim_socket(&socket)?;
    let _socket_path = SocketPath(&socket);
    listener.set_nonblocking(true)?;
    while !STOP.load(Ordering::Relaxed) {
        if poll_fd(listener.as_raw_fd(), libc::POLLIN)? & libc::POLLIN == 0 {
            continue;
        }
        match listener.accept() {
            Ok((stream, _)) => {
                let _ = serve(stream, &worker, &model, &libraries);
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

fn main() -> ExitCode {
    if run().is_ok() {
        ExitCode::SUCCESS
    } else {
        eprintln!("QA model socket unavailable");
        ExitCode::FAILURE
    }
}
