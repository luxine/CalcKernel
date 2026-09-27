use std::{
    io,
    sync::atomic::{AtomicI32, Ordering},
};

static CHILD: AtomicI32 = AtomicI32::new(0);
const PENDING: i32 = -1;
const SIGINT: i32 = 2;
const SIG_ERR: usize = usize::MAX;

pub(super) struct ForwardGuard {
    previous: usize,
}

impl ForwardGuard {
    pub(super) fn install() -> io::Result<Self> {
        CHILD.store(0, Ordering::Release);
        let previous = unsafe {
            // SAFETY: `forward_interrupt` has the C signal-handler ABI and
            // the previous handler is restored before this process exits.
            signal(SIGINT, forward_interrupt as usize)
        };
        if previous == SIG_ERR {
            return Err(io::Error::last_os_error());
        }
        Ok(Self { previous })
    }

    pub(super) fn set_child(&mut self, child: u32) -> io::Result<()> {
        let child = i32::try_from(child)
            .map_err(|_| io::Error::other("child process identifier exceeds i32"))?;
        // Registration and the handler must exchange pending interrupts through
        // one atomic: the OS child can be visible before spawn returns its PID.
        if CHILD.swap(child, Ordering::AcqRel) == PENDING {
            forward_interrupt(SIGINT);
        }
        Ok(())
    }
}

impl Drop for ForwardGuard {
    fn drop(&mut self) {
        CHILD.store(0, Ordering::Release);
        unsafe {
            // SAFETY: `previous` is the exact handler value returned by
            // the successful installation in this process.
            signal(SIGINT, self.previous);
        }
    }
}

extern "C" fn forward_interrupt(signal_number: i32) {
    let child = match CHILD.compare_exchange(0, PENDING, Ordering::AcqRel, Ordering::Acquire) {
        Ok(_) => return,
        Err(child) => child,
    };
    if child > 0 {
        unsafe {
            // SAFETY: `kill` is async-signal-safe and the positive process
            // identifier was returned by the active child handle.
            kill(child, signal_number);
        }
    }
}

unsafe extern "C" {
    fn signal(signal_number: i32, handler: usize) -> usize;
    fn kill(process: i32, signal_number: i32) -> i32;
}
