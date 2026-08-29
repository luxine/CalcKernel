use calckernel::{BoundsMode, OverflowMode};

use super::runtime_support::{executable_bytes, run_executable};

#[test]
fn executable_standalone_void_and_i32_entries_should_run_without_external_tools() {
    for (label, source, expected_status, expected_stdout) in [
        (
            "void-entry",
            "fn main() -> void { print_bool(true); }",
            0,
            "true",
        ),
        (
            "i32-entry",
            "fn main() -> i32 { print_i32(7); return 7; }",
            7,
            "7",
        ),
    ] {
        let output = run_executable(
            &executable_bytes(source, OverflowMode::Unchecked, BoundsMode::Unchecked),
            label,
        );
        assert_eq!(output.status.code(), Some(expected_status), "{label}");
        assert_eq!(output.stdout, expected_stdout.as_bytes(), "{label}");
        assert_eq!(output.stderr, b"", "{label}");
    }
}

#[test]
fn executable_checked_i32_entry_should_use_an_internal_nonnull_result_pointer() {
    let output = run_executable(
        &executable_bytes(
            "fn main() -> i32 { return 23 + 19; }",
            OverflowMode::Checked,
            BoundsMode::Checked,
        ),
        "checked-i32-entry",
    );
    assert_eq!(output.status.code(), Some(42));
    assert_eq!(output.stdout, b"");
    assert_eq!(output.stderr, b"");
}
