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
            !text.contains("PathBuf::from(\"/Users/lynn/code/CalcKernel\").join"),
            "{path} must use CALCKERNEL_TS_ROOT-aware fixture paths instead of joining the local oracle path directly"
        );
        assert!(
            !text.contains("PathBuf::from(\"/Users/lynn/code/CalcKernel/"),
            "{path} must use CALCKERNEL_TS_ROOT-aware fixture paths instead of embedding local absolute fixture paths"
        );
        assert!(
            !text.contains("const tsIndexPath = \"/Users/lynn/code/CalcKernel/"),
            "{path} must use CALCKERNEL_TS_ROOT-aware package oracle paths instead of embedding local absolute fixture paths"
        );
    }
}

#[test]
fn oracle_test_support_should_require_explicit_root_configuration() {
    let support =
        fs::read_to_string("tests/support/oracle.rs").expect("read TypeScript oracle test support");

    assert!(
        !support.contains("/Users/lynn/code/CalcKernel"),
        "shared oracle support must not fall back to a developer-specific path"
    );
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
