use std::process::Command;

pub(super) fn configure(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    // SAFETY: setpgid is async-signal-safe, touches no Rust state, and runs
    // before the child can execute the user-authorized runner.
    unsafe {
        command.pre_exec(|| {
            if libc::setpgid(0, 0) == 0 {
                Ok(())
            } else {
                Err(std::io::Error::last_os_error())
            }
        });
    }
}

pub(super) struct Containment(i32);

pub(super) fn establish(child: &std::process::Child) -> Result<Containment, std::io::Error> {
    let pid = i32::try_from(child.id()).map_err(|_| std::io::Error::other("child pid overflow"))?;
    Ok(Containment(pid))
}

impl Containment {
    pub(super) fn terminate(&self) {
        let pid = self.0;
        // SAFETY: negative pid addresses only the child-owned process group.
        unsafe {
            libc::kill(-pid, libc::SIGTERM);
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
        // SAFETY: same child-owned process group as above.
        unsafe {
            libc::kill(-pid, libc::SIGKILL);
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while std::time::Instant::now() < deadline {
            // SAFETY: signal zero only queries the child-owned process group.
            if unsafe { libc::kill(-pid, 0) } != 0 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
}
