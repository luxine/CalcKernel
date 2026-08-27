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
        assert!(stderr.contains(" map-jit=yes"), "{stderr:?}");
        assert!(stderr.contains(" thread-wx=yes"), "{stderr:?}");
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
    let parent = Command::new(env!("CARGO_BIN_EXE_ckc"))
        .args(["run", source.to_str().expect("UTF-8 fixture")])
        .env("PATH", "")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn interrupt parent");

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
    let output = parent.wait_with_output().expect("wait interrupt parent");
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
