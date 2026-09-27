use std::fs;

#[test]
fn rust_oracle_tests_should_not_hardcode_local_typescript_fixture_paths() {
    for path in [
        "tests/backend/c.rs",
        "tests/backend/llvm.rs",
        "tests/backend/wasm.rs",
        "tests/cli/commands.rs",
        "tests/ir/mir.rs",
    ] {
        let text = fs::read_to_string(path).unwrap_or_else(|error| panic!("read {path}: {error}"));

        assert!(
            !text.contains("CALCKERNEL_TS_ROOT="),
            "{path} must not configure the optional oracle root inline"
        );
    }
}

#[test]
fn oracle_test_support_should_require_explicit_root_configuration() {
    let support =
        fs::read_to_string("tests/support/oracle.rs").expect("read TypeScript oracle test support");

    assert!(
        support.contains("configured_typescript_root"),
        "shared oracle support must expose explicit root configuration"
    );

    for path in [
        "tests/cli/oracle_readiness.rs",
        "tests/performance/oracle_fixtures.rs",
    ] {
        let text = fs::read_to_string(path).unwrap_or_else(|error| panic!("read {path}: {error}"));
        assert!(
            text.contains("configured_typescript_root"),
            "{path} must skip oracle-only checks unless CALCKERNEL_TS_ROOT is configured"
        );
    }
}
