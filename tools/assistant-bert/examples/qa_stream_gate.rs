//! Test-only checkpoint worker. Pause each turn after its first real cited finding.

#[allow(dead_code)]
#[path = "../src/main.rs"]
mod worker;

use std::io;
use std::io::BufWriter;
use std::mem::MaybeUninit;

use worker::protocol::QaWorkerEvent;
use worker::protocol::QaWorkerReply;

fn block_release_signal() -> io::Result<libc::sigset_t> {
    let mut blocked = MaybeUninit::<libc::sigset_t>::uninit();
    // SAFETY: The stack buffer is valid for sigset initialization. Subsequent calls only
    // read or modify that initialized set. The worker's new threads inherit this mask.
    unsafe {
        if libc::sigemptyset(blocked.as_mut_ptr()) != 0 {
            return Err(io::Error::last_os_error());
        }
        let mut blocked = blocked.assume_init();
        if libc::sigaddset(&mut blocked, libc::SIGUSR1) != 0 {
            return Err(io::Error::last_os_error());
        }
        let error = libc::pthread_sigmask(libc::SIG_BLOCK, &blocked, std::ptr::null_mut());
        if error != 0 {
            return Err(io::Error::from_raw_os_error(error));
        }
        Ok(blocked)
    }
}

fn main() -> anyhow::Result<()> {
    let blocked = block_release_signal()?;
    let mut output = BufWriter::new(io::stdout().lock());
    worker::run(|reply| {
        worker::frame::write(&mut output, &reply)?;
        if matches!(
            reply,
            QaWorkerReply::Event {
                event: QaWorkerEvent::Finding { sequence: 1, .. }
            }
        ) {
            let mut signal = 0;
            // SAFETY: The set is initialized and this process blocked SIGUSR1 before
            // creating its input reader. The stack output remains valid through sigwait.
            let error = unsafe { libc::sigwait(&blocked, &mut signal) };
            if error != 0 {
                return Err(io::Error::from_raw_os_error(error));
            }
        }
        Ok(())
    })
}
