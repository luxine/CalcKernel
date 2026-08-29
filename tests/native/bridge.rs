use calckernel::{
    LLVM_BRIDGE_ABI_VERSION, NATIVE_ABI_VERSION, NativeStage, RUNTIME_ABI_VERSION, bridge_info,
    native_bridge_test_error, native_bridge_test_invalid_input,
};

#[test]
fn bridge_should_report_the_private_abi_version() {
    let info = bridge_info().expect("read linked LLVM bridge metadata");

    assert_eq!(info.abi_version, LLVM_BRIDGE_ABI_VERSION);
    assert_eq!(LLVM_BRIDGE_ABI_VERSION, 2);
    assert_eq!(NATIVE_ABI_VERSION, 1);
    assert_eq!(RUNTIME_ABI_VERSION, 1);
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
