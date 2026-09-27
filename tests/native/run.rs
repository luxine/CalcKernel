use std::{
    fs,
    process::{Command, Stdio},
};

use super::support::temp::unique_id;

fn source_file(source: &str) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!("ckc-run-{}", unique_id()));
    fs::create_dir_all(&root).expect("create run fixture");
    let path = root.join("program.ck");
    fs::write(&path, source).expect("write run fixture");
    path
}

#[cfg(unix)]
#[path = "../../src/cli/run/interrupt_unix.rs"]
mod interrupt;

#[cfg(unix)]
mod interrupt_handoff {
    use std::{
        io,
        os::unix::process::{CommandExt, ExitStatusExt},
        process::{Child, Command, Output, Stdio},
        thread,
        time::{Duration, Instant},
    };

    use super::interrupt;

    const CASE_ENV: &str = "CKC_TEST_INTERRUPT_HANDOFF_CASE";

    pub(super) struct OwnedProcess {
        child: Option<Child>,
        process_group: bool,
    }

    impl OwnedProcess {
        pub(super) fn spawn(command: &mut Command, process_group: bool) -> Self {
            if process_group {
                command.process_group(0);
            }
            Self {
                child: Some(command.spawn().expect("spawn owned test process")),
                process_group,
            }
        }

        pub(super) fn id(&self) -> u32 {
            self.child.as_ref().expect("live owner").id()
        }

        fn child(&mut self) -> &mut Child {
            self.child.as_mut().expect("live owner")
        }

        fn terminate_group(&self) {
            if self.process_group {
                let group = i32::try_from(self.id()).expect("owned process group fits pid_t");
                unsafe {
                    // SAFETY: This group was created for the owned child only.
                    super::kill(-group, 9);
                }
            }
        }

        pub(super) fn wait_output(mut self, timeout: Duration) -> io::Result<Output> {
            let deadline = Instant::now() + timeout;
            while self.child().try_wait()?.is_none() {
                if Instant::now() >= deadline {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "owned test process exceeded its exit deadline",
                    ));
                }
                thread::sleep(Duration::from_millis(10));
            }
            // A failed worker may leave descendants holding its output pipes.
            self.terminate_group();
            self.child.take().expect("live owner").wait_with_output()
        }
    }

    impl Drop for OwnedProcess {
        fn drop(&mut self) {
            if self.child.is_some() {
                self.terminate_group();
                let child = self.child();
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }

    fn isolated(case: &str, action: fn()) {
        if std::env::var(CASE_ENV).as_deref() == Ok(case) {
            action();
            return;
        }
        let output = OwnedProcess::spawn(
            Command::new(std::env::current_exe().expect("test executable"))
                .args([
                    "--exact",
                    &format!("run::interrupt_handoff::{case}"),
                    "--nocapture",
                ])
                .env(CASE_ENV, case)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped()),
            true,
        )
        .wait_output(Duration::from_secs(30))
        .expect("isolated interrupt regression must finish");
        assert!(output.status.success(), "{output:?}");
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("1 passed"),
            "isolated regression must execute exactly its selected test: {output:?}"
        );
    }

    fn owned_sleep() -> OwnedProcess {
        OwnedProcess::spawn(
            Command::new("/bin/sleep")
                .arg("30")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null()),
            false,
        )
    }

    fn raise_interrupt() {
        assert_eq!(
            unsafe {
                // SAFETY: The isolated worker installed the handler. raise targets
                // this thread and returns only after its handler has run.
                raise(2)
            },
            0
        );
    }

    fn assert_interrupted(child: OwnedProcess) {
        let output = child
            .wait_output(Duration::from_secs(2))
            .expect("SIGINT must reach the registered child");
        assert_eq!(output.status.signal(), Some(2));
    }

    #[test]
    fn before_child_registration() {
        isolated("before_child_registration", || {
            let mut guard = interrupt::ForwardGuard::install().expect("install handler");
            let child = owned_sleep();
            raise_interrupt();
            guard.set_child(child.id()).expect("register child");
            assert_interrupted(child);
        });
    }

    #[test]
    fn after_child_registration() {
        isolated("after_child_registration", || {
            let mut guard = interrupt::ForwardGuard::install().expect("install handler");
            let child = owned_sleep();
            guard.set_child(child.id()).expect("register child");
            raise_interrupt();
            assert_interrupted(child);
        });
    }

    #[test]
    fn repeated_pending_interrupts() {
        isolated("repeated_pending_interrupts", || {
            let mut guard = interrupt::ForwardGuard::install().expect("install handler");
            let child = owned_sleep();
            raise_interrupt();
            raise_interrupt();
            guard.set_child(child.id()).expect("register child");
            assert_interrupted(child);
        });
    }

    #[test]
    fn pending_interrupt_does_not_leak_to_next_guard() {
        isolated("pending_interrupt_does_not_leak_to_next_guard", || {
            let guard = interrupt::ForwardGuard::install().expect("install first handler");
            raise_interrupt();
            drop(guard);
            let mut guard = interrupt::ForwardGuard::install().expect("install next handler");
            let mut child = owned_sleep();
            guard.set_child(child.id()).expect("register child");
            let deadline = Instant::now() + Duration::from_millis(100);
            while Instant::now() < deadline {
                assert!(
                    child
                        .child()
                        .try_wait()
                        .expect("poll unsignaled child")
                        .is_none()
                );
                thread::sleep(Duration::from_millis(10));
            }
            raise_interrupt();
            assert_interrupted(child);
        });
    }

    unsafe extern "C" {
        fn raise(signal: i32) -> i32;
        fn waitpid(process: i32, status: *mut i32, options: i32) -> i32;
    }

    #[test]
    fn timeout_should_terminate_and_reap_owned_process() {
        let child = OwnedProcess::spawn(Command::new("/bin/sleep").arg("30"), true);
        let pid = i32::try_from(child.id()).expect("owned PID fits pid_t");
        let error = child
            .wait_output(Duration::from_millis(50))
            .expect_err("a running child must fail its deadline");
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert_eq!(
            unsafe {
                // SAFETY: signal zero only probes the owned process identifier.
                super::kill(pid, 0)
            },
            -1,
            "timed-out child must no longer exist"
        );
        assert_eq!(
            unsafe {
                // SAFETY: A null status buffer is allowed; WNOHANG never blocks.
                waitpid(pid, std::ptr::null_mut(), 1)
            },
            -1,
            "timed-out child must already have been reaped"
        );
    }
}

#[test]
fn public_run_should_self_spawn_and_preserve_program_stdio_and_status() {
    let source = source_file("fn main() -> i32 { print_i32(42); print_newline(); return 7; }");
    let output = Command::new(env!("CARGO_BIN_EXE_ckc"))
        .args(["run", source.to_str().expect("UTF-8 fixture")])
        .env("PATH", "")
        .output()
        .expect("run public parent");
    assert_eq!(output.status.code(), Some(7));
    assert_eq!(output.stdout, b"42\n");
    assert_eq!(output.stderr, b"");
}

#[test]
fn internal_jit_audit_should_report_enforced_memory_policy() {
    let source = source_file("fn main() -> i32 { print_i32(0); print_newline(); return 0; }");
    let output = Command::new(env!("CARGO_BIN_EXE_ckc"))
        .args(["run", source.to_str().expect("UTF-8 fixture"), "--no-cache"])
        .env("PATH", "")
        .env("CKC_INTERNAL_JIT_AUDIT", "1")
        .output()
        .expect("run audited public parent");
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(output.stdout, b"0\n");
    let stderr = String::from_utf8(output.stderr).expect("UTF-8 audit output");
    assert!(stderr.starts_with("CKC_JIT_AUDIT_V1 "), "{stderr:?}");
    assert!(stderr.contains(" relocation=rw-nx"), "{stderr:?}");
    assert!(stderr.contains(" code=rx"), "{stderr:?}");
    assert!(stderr.contains(" data=nx"), "{stderr:?}");
    assert!(stderr.contains(" icache=flushed"), "{stderr:?}");
    #[cfg(target_os = "macos")]
    {
        assert!(
            stderr.contains(" map-jit=yes thread-wx-supported=yes thread-wx=yes")
                || stderr.contains(" map-jit=no thread-wx-supported=no thread-wx=no"),
            "{stderr:?}"
        );
    }
}

#[test]
fn public_run_should_preserve_specific_checked_runtime_failure() {
    let source = source_file(
        "fn fail() -> i64 { let max: i64 = 9223372036854775807; return max + 1; } fn main() -> i32 { let ignored: i64 = fail(); return 0; }",
    );
    let output = Command::new(env!("CARGO_BIN_EXE_ckc"))
        .args([
            "run",
            source.to_str().expect("UTF-8 fixture"),
            "--overflow",
            "checked",
        ])
        .env("PATH", "")
        .output()
        .expect("run checked public parent");
    assert_eq!(output.status.code(), Some(240));
    assert_eq!(output.stdout, b"");
    assert_eq!(output.stderr, b"CKR0001: integer overflow\n");
}

#[test]
fn private_child_command_should_reject_direct_invocation() {
    let source = source_file("fn main() -> i32 { return 0; }");
    let output = Command::new(env!("CARGO_BIN_EXE_ckc"))
        .args([
            "__ckc-run-child",
            "00",
            source.to_str().expect("UTF-8 fixture"),
        ])
        .env_remove("CKC_PRIVATE_RUN_TOKEN")
        .output()
        .expect("probe private child command");
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(output.stdout, b"");
    assert_eq!(
        output.stderr,
        b"error: invalid private run child protocol\n"
    );
}

#[cfg(unix)]
#[test]
fn public_run_should_forward_interrupt_and_map_abnormal_child_to_ckr0006() {
    use std::{
        os::unix::process::ExitStatusExt,
        thread,
        time::{Duration, Instant},
    };

    let source = source_file("fn main() -> void { while true {} }");
    let parent = interrupt_handoff::OwnedProcess::spawn(
        Command::new(env!("CARGO_BIN_EXE_ckc"))
            .args(["run", source.to_str().expect("UTF-8 fixture")])
            .env("PATH", "")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped()),
        true,
    );

    let deadline = Instant::now() + Duration::from_secs(10);
    while !child_process_started(parent.id()) && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    assert!(
        child_process_started(parent.id()),
        "public run parent did not spawn its private child"
    );
    let result = unsafe { kill(parent.id() as i32, 2) };
    assert_eq!(result, 0, "send SIGINT to public parent");
    let output = parent
        .wait_output(Duration::from_secs(10))
        .expect("SIGINT must terminate the public parent within its deadline");
    assert_eq!(output.status.code(), Some(245), "{output:?}");
    assert_eq!(output.status.signal(), None);
    assert_eq!(output.stdout, b"");
    assert_eq!(
        output.stderr,
        b"CKR0006: native child terminated abnormally\n"
    );
}

#[cfg(target_os = "linux")]
fn child_process_started(parent: u32) -> bool {
    fs::read_to_string(format!("/proc/{parent}/task/{parent}/children"))
        .is_ok_and(|children| !children.trim().is_empty())
}

#[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
fn child_process_started(_parent: u32) -> bool {
    true
}

#[cfg(target_os = "macos")]
fn child_process_started(parent: u32) -> bool {
    let mut children = [0i32; 8];
    let bytes = unsafe {
        // SAFETY: The output buffer is writable for the exact byte count and
        // the queried process identifier came from the live child handle.
        proc_listchildpids(
            parent as i32,
            children.as_mut_ptr().cast(),
            std::mem::size_of_val(&children) as i32,
        )
    };
    bytes > 0 && children.iter().any(|child| *child > 0)
}

#[cfg(unix)]
unsafe extern "C" {
    fn kill(process: i32, signal: i32) -> i32;
}

#[cfg(target_os = "macos")]
#[link(name = "proc")]
unsafe extern "C" {
    fn proc_listchildpids(parent: i32, buffer: *mut core::ffi::c_void, buffer_size: i32) -> i32;
}
