use calckernel::{
    LLVM_BRIDGE_ABI_VERSION, NATIVE_ABI_VERSION, NativeStage, RUNTIME_ABI_VERSION, bridge_info,
    native_bridge_test_error, native_bridge_test_invalid_input,
};

#[test]
fn bridge_should_report_the_private_abi_version() {
    let info = bridge_info().expect("read linked LLVM bridge metadata");

    assert_eq!(info.abi_version, LLVM_BRIDGE_ABI_VERSION);
    assert_eq!(LLVM_BRIDGE_ABI_VERSION, 4);
    assert_eq!(NATIVE_ABI_VERSION, 1);
    assert_eq!(RUNTIME_ABI_VERSION, 2);
}

#[test]
fn bridge_abi_4_should_define_owned_late_layout_plan_and_report_surface() {
    let header = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("native/bridge/ckc_llvm.h"),
    )
    .expect("bridge header");
    assert!(header.contains("#define CKC_LLVM_BRIDGE_ABI_VERSION 4u"));
    assert!(header.contains("typedef struct CkcLlvmLateLayoutReport"));
    assert!(header.contains("uint8_t pre_layout_digest[32]"));
    assert!(header.contains("uint8_t post_structural_digest[32]"));
    assert!(header.contains("ckc_llvm_module_apply_late_layout"));
    assert_eq!(LLVM_BRIDGE_ABI_VERSION, 4);
}

#[test]
fn bridge_should_report_pinned_llvm_version() {
    let info = bridge_info().expect("read linked LLVM bridge metadata");

    assert_eq!(info.llvm_version, "22.1.8");
}

#[test]
fn bridge_should_report_a_nonempty_host_triple() {
    let info = bridge_info().expect("read linked LLVM bridge metadata");

    assert!(!info.host_triple.is_empty());
}

#[test]
fn bridge_should_return_owned_injected_errors() {
    let error = native_bridge_test_error();

    assert_eq!(
        error.to_string(),
        "LLVM bridge failed with code 3: injected LLVM bridge failure"
    );
}

#[test]
fn bridge_should_return_typed_invalid_input_instead_of_unwinding() {
    let error = native_bridge_test_invalid_input();

    assert_eq!(error.stage, NativeStage::Context);
    assert_eq!(error.code, 1);
    assert!(error.message.contains("output is null"), "{error}");
}

#[cfg(unix)]
#[test]
fn bridge_should_isolate_windows_sdk_macros_from_llvm_and_std_names() {
    use std::{path::PathBuf, process::Command};

    let root = super::support::oracle::repo_root();
    let prefix = PathBuf::from(std::env::var_os("CKC_LLVM_PREFIX").expect("LLVM prefix"));
    for preexisting_minmax in [false, true] {
        // Compile the actual bridge's COFF branch against the real LLVM headers.
        // Only Windows macros/process declarations are simulated; the two real
        // Windows jobs remain responsible for SDK, MSVC, linking and ABI checks.
        let mut command = Command::new("c++");
        command.args(["-std=c++20", "-fsyntax-only", "-DCKC_LLD_COFF"]);
        if preexisting_minmax {
            command.arg("-DCKC_TEST_PREEXISTING_MINMAX");
        }
        let output = command
            .arg("-I")
            .arg(root.join("tests/fixtures/native/windows-header"))
            .arg("-I")
            .arg(prefix.join("include"))
            .arg("-I")
            .arg(root.join("native/bridge"))
            .arg(root.join("native/bridge/ckc_llvm.cpp"))
            .output()
            .expect("compile COFF bridge macro-surface regression");
        assert!(
            output.status.success(),
            "preexisting_minmax={preexisting_minmax}:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
