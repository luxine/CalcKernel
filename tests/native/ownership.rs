use calckernel::{
    NativeContext, NativeJit, NativeModule, NativeTarget, NativeToolchain, OrcObjectLayer,
    native_bridge_test_error,
};

#[test]
fn native_context_should_create_and_drop_repeatedly() {
    for _ in 0..128 {
        let context = NativeContext::new().expect("create LLVM context");
        drop(context);
    }
}

#[test]
fn native_target_should_create_and_drop_repeatedly() {
    for _ in 0..64 {
        let target = NativeTarget::host().expect("create host target machine");
        drop(target);
    }
}

#[test]
fn native_module_and_object_should_create_and_drop_repeatedly() {
    for _ in 0..32 {
        let context = NativeContext::new().expect("create LLVM context");
        let mut module = NativeModule::empty(&context).expect("create empty LLVM module");
        let target = NativeTarget::host().expect("create host target machine");
        let object = target
            .emit_object(&mut module)
            .expect("emit verified empty object");
        assert!(!object.is_empty());
        drop(object);
        drop(module);
        drop(target);
        drop(context);
    }
}

#[test]
fn injected_middle_stage_error_should_preserve_live_owner_relationships() {
    let context = NativeContext::new().expect("create LLVM context");
    let mut module = NativeModule::empty(&context).expect("create empty LLVM module");
    let target = NativeTarget::host().expect("create host target machine");

    let injected = native_bridge_test_error();
    assert_eq!(injected.stage.to_string(), "LLVM bridge");

    let object = target
        .emit_object(&mut module)
        .expect("owners remain valid after injected bridge error");
    assert!(!object.is_empty());
}

#[test]
fn native_jit_should_create_and_drop_repeatedly() {
    for _ in 0..32 {
        let jit = NativeJit::new().expect("create empty LLJIT");
        drop(jit);
    }
}

#[test]
fn native_jit_should_use_jitlink_on_macos_aarch64() {
    let jit = NativeJit::new().expect("create empty LLJIT");

    assert_eq!(jit.object_layer(), OrcObjectLayer::JitLink);
}

#[test]
fn native_toolchain_should_own_context_target_and_jit() {
    let toolchain = NativeToolchain::new().expect("create native toolchain owners");

    assert_eq!(toolchain.object_layer(), OrcObjectLayer::JitLink);
}
