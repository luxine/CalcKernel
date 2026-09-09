#[test]
fn kir_multiversion_schema_and_explicit_target_bridge_should_be_closed_and_versioned() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let model = std::fs::read_to_string(root.join("src/ir/kir/multiversion.rs"))
        .expect("multiversion KIR model");
    let header =
        std::fs::read_to_string(root.join("native/bridge/ckc_llvm.h")).expect("LLVM bridge header");
    let bridge = std::fs::read_to_string(root.join("native/bridge/ckc_llvm.cpp"))
        .expect("LLVM bridge source");

    assert!(model.contains("KIR_MULTIVERSION_TARGET_SET_SCHEMA: u16 = 1"));
    assert!(model.contains("KIR_MULTIVERSION_BUNDLE_SCHEMA: u16 = 1"));
    assert!(model.contains("X86_64V3"));
    assert!(model.contains("X86_64V4"));
    assert!(model.contains("AArch64Sve2"));
    assert!(header.contains("ckc_llvm_target_create_explicit"));
    assert!(bridge.contains("explicit LLVM feature target must match the build host ABI"));
}
