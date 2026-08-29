use std::{fs, process, process::Command};

use calckernel::{BoundsMode, OverflowMode};

use super::runtime_support::{executable_bytes, run_executable, write_executable};

#[test]
fn runtime_should_format_integer_boolean_newline_and_f64_spellings_byte_exactly() {
    let source = r#"
fn min_i64() -> i64 { let max: i64 = 9223372036854775807; return -max - 1; }
fn max_i64() -> i64 { let value: i64 = 9223372036854775807; return value; }
fn max_u32() -> u32 { let value: u32 = 4294967295; return value; }
fn max_u64() -> u64 { let value: u64 = 18446744073709551615; return value; }
fn main() -> void {
  print_i32(-2147483647 - 1); print_newline();
  print_i32(2147483647); print_newline();
  print_i64(min_i64()); print_newline();
  print_i64(max_i64()); print_newline();
  print_u32(max_u32()); print_newline();
  print_u64(max_u64()); print_newline();
  print_bool(true); print_newline();
  print_bool(false); print_newline();
  print_f64(1.5); print_newline();
  print_f64(0.1); print_newline();
  print_f64(-0.0); print_newline();
  print_f64(1.0 / 0.0); print_newline();
  print_f64(-1.0 / 0.0); print_newline();
  print_f64(0.0 / 0.0); print_newline();
}
"#;
    let output = run_executable(
        &executable_bytes(source, OverflowMode::Unchecked, BoundsMode::Unchecked),
        "runtime-spellings",
    );
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stderr, b"");
    assert_eq!(
        output.stdout,
        b"-2147483648\n2147483647\n-9223372036854775808\n9223372036854775807\n4294967295\n18446744073709551615\ntrue\nfalse\n1.5\n0.1\n-0.0\ninf\n-inf\nnan\n"
    );
}

#[test]
fn every_generated_finite_f64_spelling_should_round_trip_to_identical_bits() {
    let mut state = 0x6a09_e667_f3bc_c909u64;
    let mut values = vec![
        f64::MIN_POSITIVE,
        f64::from_bits(1),
        f64::MAX,
        -f64::MAX,
        1.0000000000000002,
        2.2250738585072014e-308,
    ];
    while values.len() < 192 {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        let value = f64::from_bits(state);
        if value.is_finite() && value != 0.0 {
            values.push(value);
        }
    }
    let mut source = String::from("fn main() -> void {\n");
    for value in &values {
        source.push_str(&format!("print_f64({value:.17e}); print_newline();\n"));
    }
    source.push_str("}\n");
    let output = run_executable(
        &executable_bytes(&source, OverflowMode::Unchecked, BoundsMode::Unchecked),
        "runtime-f64-roundtrip",
    );
    assert_eq!(output.status.code(), Some(0), "{:?}", output.stderr);
    let text = String::from_utf8(output.stdout).expect("runtime f64 UTF-8");
    let spellings = text.lines().collect::<Vec<_>>();
    assert_eq!(spellings.len(), values.len());
    for (value, spelling) in values.iter().zip(spellings) {
        assert!(!spelling.contains('E'));
        assert!(!spelling.contains("e+"));
        let parsed = spelling.parse::<f64>().expect("parse runtime f64");
        assert_eq!(parsed.to_bits(), value.to_bits(), "{value:?} -> {spelling}");
    }
}

#[test]
fn checked_runtime_failures_should_use_exact_messages_and_reserved_statuses() {
    for (label, source, expected_status, expected_stderr) in [
        (
            "overflow",
            "fn fail() -> i64 { let value: i64 = 9223372036854775807; return value + 1; } fn main() -> i32 { let ignored: i64 = fail(); return 0; }",
            240,
            "CKR0001: integer overflow\n",
        ),
        (
            "division",
            "fn fail() -> i64 { let one: i64 = 1; let zero: i64 = 0; return one / zero; } fn main() -> i32 { let ignored: i64 = fail(); return 0; }",
            241,
            "CKR0002: integer division or modulo by zero\n",
        ),
    ] {
        let output = run_executable(
            &executable_bytes(source, OverflowMode::Checked, BoundsMode::Checked),
            label,
        );
        assert_eq!(output.status.code(), Some(expected_status), "{label}");
        assert_eq!(output.stdout, b"");
        assert_eq!(output.stderr, expected_stderr.as_bytes());
    }

    let runtime_source = include_str!("../../native/runtime/common/runtime.c");
    for message in [
        "CKR0001: integer overflow\\n",
        "CKR0002: integer division or modulo by zero\\n",
        "CKR0003: null checked result pointer\\n",
        "CKR0004: slice index or sub-slice out of bounds\\n",
        "CKR0005: standard output write failed\\n",
        "CKR0006: native child terminated abnormally\\n",
        "CKR0007: unsafe contract violation\\n",
    ] {
        assert!(runtime_source.contains(message), "missing {message}");
    }
    for exit_status in 240..=245 {
        assert!(
            runtime_source.contains(&format!(", {exit_status}}}")),
            "missing exit status {exit_status}"
        );
    }
    assert!(runtime_source.contains(", 246}"), "missing exit status 246");
}

#[cfg(unix)]
#[test]
fn closed_standard_output_should_attempt_ckr0005_on_stderr_and_exit_244() {
    use std::os::unix::process::CommandExt;

    unsafe extern "C" {
        fn close(file_descriptor: i32) -> i32;
    }

    let bytes = executable_bytes(
        "fn main() -> void { print_i32(42); }",
        OverflowMode::Unchecked,
        BoundsMode::Unchecked,
    );
    let path = write_executable(&bytes, "runtime-stdout-failure");
    let mut command = Command::new(&path);
    command
        .env("PATH", "")
        .stdout(process::Stdio::piped())
        .stderr(process::Stdio::piped());
    unsafe {
        // SAFETY: `pre_exec` runs after the child's stdio descriptors have
        // been installed and before the CK executable starts. Closing only
        // descriptor 1 makes the write failure deterministic without racing
        // the child against a parent-side pipe close.
        command.pre_exec(|| {
            if close(1) == 0 {
                Ok(())
            } else {
                Err(std::io::Error::last_os_error())
            }
        });
    }
    let output = command.output().expect("run stdout failure executable");
    fs::remove_file(path).expect("remove stdout failure executable");
    assert_eq!(output.status.code(), Some(244), "{output:?}");
    assert_eq!(output.stderr, b"CKR0005: standard output write failed\n");
}
