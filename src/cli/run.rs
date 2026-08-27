use std::{
    fs::File,
    io::{self, Read, Write},
    process::{Command, ExitStatus, Stdio},
};

use calckernel::{NativeJit, OrcObjectLayer};

use super::{args::ParsedArgs, commands::compile_run_object};

const PRIVATE_TOKEN_ENV: &str = "CKC_PRIVATE_RUN_TOKEN";
const INTERNAL_JIT_AUDIT_ENV: &str = "CKC_INTERNAL_JIT_AUDIT";
const ABNORMAL_MESSAGE: &[u8] = b"CKR0006: native child terminated abnormally\n";

pub(super) fn run_public_parent(args: &[String]) -> i32 {
    match run_public_parent_inner(args) {
        Ok(status) => status,
        Err(message) => {
            write_error(&message);
            1
        }
    }
}

fn run_public_parent_inner(args: &[String]) -> Result<i32, String> {
    let mut interrupt = interrupt::ForwardGuard::install()
        .map_err(|error| format!("failed to install run interrupt forwarding: {error}"))?;
    let executable = std::env::current_exe()
        .map_err(|error| format!("failed to resolve current ckc executable: {error}"))?;
    let token = private_token()?;
    let mut command = Command::new(executable);
    command
        .arg("__ckc-run-child")
        .arg(&token)
        .args(args)
        .env(PRIVATE_TOKEN_ENV, &token)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    let mut child = command
        .spawn()
        .map_err(|error| format!("failed to start private ckc run child: {error}"))?;
    if let Err(error) = interrupt.set_child(child.id()) {
        let _ = child.kill();
        let _ = child.wait();
        return Err(format!("failed to arm run interrupt forwarding: {error}"));
    }
    let status = child
        .wait()
        .map_err(|error| format!("failed to wait for private ckc run child: {error}"))?;
    drop(interrupt);
    Ok(classify_child_status(status))
}

pub(super) fn run_private_child(args: &[String]) -> i32 {
    let Some((token, user_args)) = args.split_first() else {
        write_private_protocol_error();
        return 2;
    };
    let valid = token.len() == 64
        && token.bytes().all(|byte| byte.is_ascii_hexdigit())
        && std::env::var(PRIVATE_TOKEN_ENV).as_deref() == Ok(token.as_str());
    if !valid {
        write_private_protocol_error();
        return 2;
    }

    let parsed = match ParsedArgs::parse("run", user_args) {
        Ok(parsed) => parsed,
        Err(message) => {
            write_error(&message);
            return 2;
        }
    };
    let object = match compile_run_object(&parsed) {
        Ok(object) => object,
        Err(message) => {
            write_error(&message);
            return 1;
        }
    };
    let mut jit = match NativeJit::new() {
        Ok(jit) => jit,
        Err(error) => {
            write_error(&error.to_string());
            return 1;
        }
    };
    match jit.execute_entry(&object) {
        Ok(status) => {
            if std::env::var(INTERNAL_JIT_AUDIT_ENV).as_deref() == Ok("1")
                && let Err(message) = emit_jit_memory_audit(&jit)
            {
                write_error(&message);
                return 1;
            }
            status
        }
        Err(error) => {
            write_error(&error.to_string());
            1
        }
    }
}

fn emit_jit_memory_audit(jit: &NativeJit) -> Result<(), String> {
    let audit = jit.memory_audit().map_err(|error| error.to_string())?;
    let common_policy = audit.allocations > 0
        && audit.relocation_write_non_execute
        && audit.final_code_read_execute
        && audit.final_data_non_execute
        && audit.instruction_cache_finalizations > 0;
    let darwin_policy = !cfg!(target_os = "macos")
        || (audit.darwin_map_jit
            && audit.darwin_thread_write_protection_supported
            && audit.darwin_thread_write_protection);
    if !common_policy || !darwin_policy {
        return Err(format!("internal JIT memory audit failed: {audit:?}"));
    }

    let layer = match jit.object_layer() {
        OrcObjectLayer::JitLink => "jitlink",
        OrcObjectLayer::RuntimeDyldCoffAarch64 => "rtdyld-coff-aarch64",
    };
    let yes_no = |value| if value { "yes" } else { "no" };
    let mut stderr = io::stderr().lock();
    writeln!(
        stderr,
        "CKC_JIT_AUDIT_V1 layer={layer} allocations={} relocation=rw-nx code=rx data=nx icache=flushed icache-count={} map-jit={} thread-wx={}",
        audit.allocations,
        audit.instruction_cache_finalizations,
        yes_no(audit.darwin_map_jit),
        yes_no(audit.darwin_thread_write_protection),
    )
    .map_err(|error| format!("failed to write internal JIT memory audit: {error}"))
}

fn private_token() -> Result<String, String> {
    let mut bytes = [0u8; 32];
    fill_random(&mut bytes)
        .map_err(|error| format!("failed to create private run token: {error}"))?;
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut token = String::with_capacity(64);
    for byte in bytes {
        token.push(char::from(HEX[usize::from(byte >> 4)]));
        token.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    Ok(token)
}

#[cfg(unix)]
fn fill_random(bytes: &mut [u8]) -> io::Result<()> {
    File::open("/dev/urandom")?.read_exact(bytes)
}

#[cfg(target_os = "windows")]
fn fill_random(bytes: &mut [u8]) -> io::Result<()> {
    let status = unsafe {
        // SAFETY: `bytes` is writable for the provided length and the system
        // preferred RNG mode requires no algorithm handle.
        BCryptGenRandom(
            std::ptr::null_mut(),
            bytes.as_mut_ptr(),
            bytes.len() as u32,
            2,
        )
    };
    if status == 0 {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "BCryptGenRandom returned NTSTATUS 0x{:08x}",
            status as u32
        )))
    }
}

fn classify_child_status(status: ExitStatus) -> i32 {
    if is_abnormal(status) {
        let _ = io::stderr().write_all(ABNORMAL_MESSAGE);
        245
    } else {
        status.code().unwrap_or(245)
    }
}

#[cfg(unix)]
fn is_abnormal(status: ExitStatus) -> bool {
    use std::os::unix::process::ExitStatusExt;
    status.signal().is_some()
}

#[cfg(target_os = "windows")]
fn is_abnormal(status: ExitStatus) -> bool {
    matches!(
        status.code().map(|code| code as u32),
        Some(0xc000_0005 | 0xc000_001d | 0xc000_0094 | 0xc000_0096 | 0xc000_00fd | 0x4001_0004)
    )
}

fn write_private_protocol_error() {
    let _ = io::stderr().write_all(b"error: invalid private run child protocol\n");
}

fn write_error(message: &str) {
    let mut stderr = io::stderr().lock();
    let _ = stderr.write_all(message.as_bytes());
    if !message.ends_with('\n') {
        let _ = stderr.write_all(b"\n");
    }
}

#[cfg(target_os = "windows")]
#[link(name = "bcrypt")]
unsafe extern "system" {
    fn BCryptGenRandom(
        algorithm: *mut core::ffi::c_void,
        bytes: *mut u8,
        length: u32,
        flags: u32,
    ) -> i32;
}

#[cfg(unix)]
mod interrupt {
    use std::{
        io,
        sync::atomic::{AtomicI32, Ordering},
    };

    static CHILD: AtomicI32 = AtomicI32::new(0);
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
            CHILD.store(child, Ordering::Release);
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
        let child = CHILD.load(Ordering::Acquire);
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
}

#[cfg(target_os = "windows")]
mod interrupt {
    use std::io;

    pub(super) struct ForwardGuard;

    impl ForwardGuard {
        pub(super) fn install() -> io::Result<Self> {
            let installed = unsafe {
                // SAFETY: The handler has the Windows console-control ABI and
                // remains valid until the guard unregisters it.
                SetConsoleCtrlHandler(Some(handle_control), 1)
            };
            if installed == 0 {
                Err(io::Error::last_os_error())
            } else {
                Ok(Self)
            }
        }

        pub(super) fn set_child(&mut self, _child: u32) -> io::Result<()> {
            Ok(())
        }
    }

    impl Drop for ForwardGuard {
        fn drop(&mut self) {
            unsafe {
                // SAFETY: This removes the same process-global handler that
                // the guard installed.
                SetConsoleCtrlHandler(Some(handle_control), 0);
            }
        }
    }

    unsafe extern "system" fn handle_control(kind: u32) -> i32 {
        i32::from(matches!(kind, 0 | 1))
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn SetConsoleCtrlHandler(
            handler: Option<unsafe extern "system" fn(u32) -> i32>,
            add: i32,
        ) -> i32;
    }
}
