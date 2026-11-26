//! Private Unix socket listener. LibTorch is linked only by the child executable.

#[allow(dead_code)]
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
use std::io::Read;
use std::io::Write;
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
use std::time::Instant;

use anyhow::anyhow;
use anyhow::ensure;
use anyhow::Result;
use serde::de::DeserializeOwned;

use protocol::qa_frame_len;
use protocol::QaModelTarget;
use protocol::QaWorkerCommand;
use protocol::QaWorkerEvent;
use protocol::QaWorkerReply;
use protocol::QA_CONTEXT_USAGE_ENV;
use protocol::QA_DIALOGUE_ENV;
use protocol::QA_MAX_FRAME_BYTES;
use protocol::QA_MODEL_CONTEXT_ENV;
use protocol::QA_MODEL_REVISION;
use protocol::QA_MODEL_TARGET_ENV;
use protocol::QA_RESPONSE_PLAN_ENV;
use protocol::QA_STREAMING_ENV;
use protocol::QA_WORKER_PROTOCOL;

const INITIAL_FRAME_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_HANDSHAKE_BYTES: usize = 128;
const EXECUTOR_LEASE: Duration = Duration::from_secs(30);
const SHUTDOWN_GRACE: Duration = Duration::from_secs(1);
const HEADER_BYTES: usize = 4;
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

fn interests(stream: &impl AsRawFd, events: libc::c_short) -> libc::pollfd {
    libc::pollfd {
        fd: stream.as_raw_fd(),
        events,
        revents: 0,
    }
}

fn poll(descriptors: &mut [libc::pollfd]) -> io::Result<()> {
    // SAFETY: poll sees every initialized writable pollfd for the duration of the call.
    if unsafe {
        libc::poll(
            descriptors.as_mut_ptr(),
            descriptors.len() as libc::nfds_t,
            POLL_MILLIS,
        )
    } < 0
    {
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::Interrupted {
            return Err(error);
        }
    }
    Ok(())
}

struct ModelChild(Child);

impl ModelChild {
    fn reap(&mut self) -> io::Result<()> {
        if self.0.try_wait()?.is_none() {
            if let Ok(pid) = i32::try_from(self.0.id()) {
                // SAFETY: An unreaped child owns its PID and was spawned as process group leader.
                unsafe { libc::kill(-pid, libc::SIGKILL) };
            }
            let _ = self.0.kill();
        }
        self.0.wait()?;
        Ok(())
    }
}

impl Drop for ModelChild {
    fn drop(&mut self) {
        let _ = self.reap();
    }
}

/// Assemble at most one bounded frame from a nonblocking socket.
struct Incoming {
    bytes: Vec<u8>,
    received: usize,
}

impl Default for Incoming {
    fn default() -> Self {
        Self {
            bytes: vec![0; HEADER_BYTES],
            received: 0,
        }
    }
}

impl Incoming {
    fn read<T: DeserializeOwned>(
        &mut self,
        mut stream: &UnixStream,
        limit: usize,
    ) -> Result<Option<(T, Outgoing)>> {
        loop {
            match stream.read(&mut self.bytes[self.received..]) {
                Ok(0) => return Err(io::Error::from(io::ErrorKind::UnexpectedEof).into()),
                Ok(count) => self.received += count,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(None),
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error.into()),
            }
            if self.received != self.bytes.len() {
                continue;
            }
            if self.received == HEADER_BYTES {
                let size = qa_frame_len(self.bytes[..HEADER_BYTES].try_into()?)?;
                ensure!(size <= limit, "QA listener frame exceeds its limit");
                self.bytes.resize(HEADER_BYTES + size, 0);
                continue;
            }
            let bytes = std::mem::take(self).bytes;
            let value = serde_json::from_slice(&bytes[HEADER_BYTES..])
                .map_err(|_| anyhow!("invalid QA listener frame"))?;
            return Ok(Some((value, Outgoing { bytes, sent: 0 })));
        }
    }
}

struct Outgoing {
    bytes: Vec<u8>,
    sent: usize,
}

impl Outgoing {
    fn write(&mut self, mut stream: &UnixStream) -> io::Result<bool> {
        while self.sent < self.bytes.len() {
            match stream.write(&self.bytes[self.sent..]) {
                Ok(0) => return Err(io::Error::from(io::ErrorKind::WriteZero)),
                Ok(count) => self.sent += count,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(false),
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error),
            }
        }
        Ok(true)
    }
}

fn flush(pending: &mut Option<Outgoing>, stream: &UnixStream) -> io::Result<()> {
    if let Some(packet) = pending {
        if packet.write(stream)? {
            *pending = None;
        }
    }
    Ok(())
}

fn opening_command(stream: &UnixStream) -> Result<QaWorkerCommand> {
    let until = Instant::now() + INITIAL_FRAME_TIMEOUT;
    let mut incoming = Incoming::default();
    while !STOP.load(Ordering::Relaxed) && Instant::now() < until {
        if let Some((command, _)) = incoming.read(stream, MAX_HANDSHAKE_BYTES)? {
            return Ok(command);
        }
        poll(&mut [interests(stream, libc::POLLIN)])?;
    }
    Err(anyhow!("QA executor handshake expired"))
}

fn confirm_stopped(stream: &UnixStream) -> Result<()> {
    let mut bytes = Vec::new();
    frame::write(&mut bytes, &QaWorkerReply::Stopped)?;
    let mut stopped = Outgoing { bytes, sent: 0 };
    let until = Instant::now() + INITIAL_FRAME_TIMEOUT;
    while Instant::now() < until {
        if stopped.write(stream)? {
            return Ok(());
        }
        poll(&mut [interests(stream, libc::POLLOUT)])?;
    }
    Err(anyhow!("QA unload confirmation expired"))
}

#[derive(Eq, PartialEq)]
enum Phase {
    Starting,
    Idle,
    Pinging,
    Answering,
    Stopping,
}

fn relay(
    executor: &UnixStream,
    model: &UnixStream,
    child: &mut ModelChild,
    lease: Duration,
    reply: &mut Option<Outgoing>,
) -> Result<()> {
    let mut commands = Incoming::default();
    let mut replies = Incoming::default();
    let mut outbound = None;
    let mut phase = Phase::Starting;
    let mut expires = Instant::now() + lease;
    while !STOP.load(Ordering::Relaxed) && Instant::now() < expires {
        if child.0.try_wait()?.is_some() {
            break;
        }
        if phase != Phase::Stopping {
            if let Some((command, packet)) = commands.read(executor, QA_MAX_FRAME_BYTES)? {
                let now = Instant::now();
                if now >= expires {
                    break;
                }
                match command {
                    QaWorkerCommand::Ping if phase == Phase::Idle => {
                        phase = Phase::Pinging;
                        expires = now + lease;
                    }
                    QaWorkerCommand::Ask { request } if phase == Phase::Idle => {
                        request.validate()?;
                        phase = Phase::Answering;
                        expires = now + lease;
                    }
                    QaWorkerCommand::Shutdown => {
                        phase = Phase::Stopping;
                        expires = expires.min(now + SHUTDOWN_GRACE);
                        if outbound.is_some() {
                            // A partly written request cannot be replaced with a Shutdown frame.
                            break;
                        }
                    }
                    _ => return Err(anyhow!("unexpected QA executor command")),
                }
                outbound = Some(packet);
            }
        }
        flush(&mut outbound, model)?;
        flush(reply, executor)?;
        if reply.is_none() {
            if let Some((value, packet)) =
                replies.read::<QaWorkerReply>(model, QA_MAX_FRAME_BYTES)?
            {
                match (value, &phase) {
                    (_, Phase::Stopping) => {}
                    (
                        QaWorkerReply::Ready {
                            protocol,
                            model_revision,
                        }
                        | QaWorkerReply::ReadyWithDialogue {
                            protocol,
                            model_revision,
                        }
                        | QaWorkerReply::ReadyWithStreaming {
                            protocol,
                            model_revision,
                        }
                        | QaWorkerReply::ReadyWithStreamingContext {
                            protocol,
                            model_revision,
                            ..
                        }
                        | QaWorkerReply::ReadyWithResponsePlan {
                            protocol,
                            model_revision,
                            ..
                        },
                        Phase::Starting | Phase::Pinging,
                    ) if protocol == QA_WORKER_PROTOCOL
                        && (model_revision == QA_MODEL_REVISION
                            || QaModelTarget::ALL
                                .into_iter()
                                .any(|target| model_revision == target.revision())) =>
                    {
                        phase = Phase::Idle;
                        *reply = Some(packet);
                    }
                    (
                        QaWorkerReply::Event {
                            event: QaWorkerEvent::Complete { .. },
                        },
                        Phase::Answering,
                    ) => {
                        phase = Phase::Idle;
                        *reply = Some(packet);
                    }
                    (
                        QaWorkerReply::Event {
                            event: QaWorkerEvent::Finding { .. } | QaWorkerEvent::Delta { .. },
                        },
                        Phase::Answering,
                    ) => {
                        *reply = Some(packet);
                    }
                    _ => return Err(anyhow!("unexpected QA model reply")),
                }
            }
        }
        let mut executor_events = if phase == Phase::Stopping {
            0
        } else {
            libc::POLLIN
        };
        if reply.is_some() {
            executor_events |= libc::POLLOUT;
        }
        let mut model_events = if reply.is_none() { libc::POLLIN } else { 0 };
        if outbound.is_some() {
            model_events |= libc::POLLOUT;
        }
        let mut descriptors = [
            interests(executor, executor_events),
            interests(model, model_events),
        ];
        poll(&mut descriptors)?;
        if descriptors[0].revents & (libc::POLLHUP | libc::POLLERR | libc::POLLNVAL) != 0 {
            break;
        }
    }
    Ok(())
}

fn supervise(
    stream: &UnixStream,
    model: &UnixStream,
    child: &mut ModelChild,
    lease: Duration,
) -> Result<()> {
    let mut reply = None;
    let relayed = relay(stream, model, child, lease, &mut reply);
    child.reap()?;
    if reply.is_none_or(|packet| packet.sent == 0) {
        // A partially written frame makes this connection unusable. A fresh executor may query
        // Shutdown on the idle listener and will still get confirmation after reaping.
        confirm_stopped(stream)?;
    }
    relayed
}

fn serve(stream: UnixStream, worker: &Path, model: &Path, libraries: &Path) -> Result<()> {
    stream.set_nonblocking(true)?;
    let (report_context, dialogue, streaming) = match opening_command(&stream)? {
        QaWorkerCommand::Shutdown => return confirm_stopped(&stream),
        QaWorkerCommand::Open => (false, false, false),
        QaWorkerCommand::OpenWithContext => (true, false, false),
        QaWorkerCommand::OpenWithDialogue => (true, true, false),
        QaWorkerCommand::OpenWithStreaming => (true, true, true),
        _ => return Err(anyhow!("invalid QA executor handshake")),
    };

    // Relay rather than passing the executor FD to the worker so a stuck worker cannot hold
    // its own lease open. Each direction holds at most one bounded frame awaiting delivery.
    let (model_io, child_io) = UnixStream::pair()?;
    model_io.set_nonblocking(true)?;
    let input = OwnedFd::from(child_io.try_clone()?);
    let output = OwnedFd::from(child_io);
    let library_variable = if cfg!(target_os = "macos") {
        "DYLD_LIBRARY_PATH"
    } else {
        "LD_LIBRARY_PATH"
    };
    let mut command = Command::new(worker);
    command
        .arg("--model-dir")
        .arg(model)
        .current_dir(model)
        .env_clear()
        .env(library_variable, libraries)
        .env("OMP_NUM_THREADS", "1")
        .env("MKL_NUM_THREADS", "1")
        .env("RAYON_NUM_THREADS", "1")
        .stdin(Stdio::from(input))
        .stdout(Stdio::from(output))
        .stderr(Stdio::null())
        .process_group(0);
    if report_context {
        command.env(QA_CONTEXT_USAGE_ENV, "1");
    }
    if dialogue {
        command.env(QA_DIALOGUE_ENV, "1");
    }
    if streaming {
        command.env(QA_STREAMING_ENV, "1");
        if env::var_os(QA_MODEL_CONTEXT_ENV).as_deref() == Some("1".as_ref()) {
            command.env(QA_MODEL_CONTEXT_ENV, "1");
        }
        if env::var_os(QA_RESPONSE_PLAN_ENV).as_deref() == Some("1".as_ref()) {
            command.env(QA_RESPONSE_PLAN_ENV, "1");
        }
    }
    if let Some(target) = env::var_os(QA_MODEL_TARGET_ENV) {
        command.env(QA_MODEL_TARGET_ENV, target);
    }
    let mut child = ModelChild(command.spawn()?);
    supervise(&stream, &model_io, &mut child, EXECUTOR_LEASE)
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
        let mut descriptors = [interests(&listener, libc::POLLIN)];
        poll(&mut descriptors)?;
        if descriptors[0].revents & libc::POLLIN == 0 {
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

#[cfg(test)]
mod tests {
    use std::os::unix::process::ExitStatusExt;
    use std::thread;

    use protocol::QaPassage;
    use protocol::QaWorkerRequest;

    use super::*;

    #[test]
    fn reaps_stuck_prediction_even_when_both_socket_peers_remain_open() {
        let (mut executor, exec_io) = UnixStream::pair().unwrap();
        let (mut model, model_io) = UnixStream::pair().unwrap();
        let timeout = Duration::from_secs(5);
        executor.set_read_timeout(Some(timeout)).unwrap();
        model.set_read_timeout(Some(timeout)).unwrap();
        exec_io.set_nonblocking(true).unwrap();
        model_io.set_nonblocking(true).unwrap();
        let mut child = ModelChild(
            Command::new("/bin/sleep")
                .arg("60")
                .process_group(0)
                .spawn()
                .unwrap(),
        );
        let monitor = thread::spawn(move || {
            supervise(&exec_io, &model_io, &mut child, Duration::from_secs(1)).unwrap();
            child.0.try_wait().unwrap().unwrap()
        });

        frame::write(
            &mut model,
            &QaWorkerReply::Ready {
                protocol: QA_WORKER_PROTOCOL,
                model_revision: QA_MODEL_REVISION.into(),
            },
        )
        .unwrap();
        assert!(matches!(
            frame::read::<QaWorkerReply>(&mut executor, QA_MAX_FRAME_BYTES).unwrap(),
            Some(QaWorkerReply::Ready { .. })
        ));
        frame::write(
            &mut executor,
            &QaWorkerCommand::Ask {
                request: QaWorkerRequest::new(
                    "turn-a".into(),
                    "snapshot-a".into(),
                    "Is search ready?".into(),
                    vec![QaPassage::new(
                        "search".into(),
                        "Actor search is ready.".into(),
                    )],
                ),
            },
        )
        .unwrap();
        assert!(matches!(
            frame::read::<QaWorkerCommand>(&mut model, QA_MAX_FRAME_BYTES).unwrap(),
            Some(QaWorkerCommand::Ask { .. })
        ));

        // There is no Shutdown and neither peer is closed when the on-time reap is confirmed.
        assert!(matches!(
            frame::read::<QaWorkerReply>(&mut executor, QA_MAX_FRAME_BYTES).unwrap(),
            Some(QaWorkerReply::Stopped)
        ));
        assert_eq!(monitor.join().unwrap().signal(), Some(libc::SIGKILL));
    }
}
