use calckernel::{
    NativeContext, NativeJit, NativeModule, NativeOptimizationLevel, NativeTarget, NativeToolchain,
    OrcObjectLayer, native_bridge_test_error,
};

fn expected_host_object_layer() -> OrcObjectLayer {
    if cfg!(all(target_os = "windows", target_arch = "aarch64")) {
        OrcObjectLayer::RuntimeDyldCoffAarch64
    } else {
        OrcObjectLayer::JitLink
    }
}

fn empty_object(context: &NativeContext, target: &NativeTarget) -> calckernel::NativeObject {
    let module = NativeModule::empty(context).expect("create empty LLVM module");
    let optimized = module
        .verify()
        .expect("verify empty LLVM module")
        .audit()
        .expect("audit empty LLVM module")
        .optimize(target, NativeOptimizationLevel::O0)
        .expect("optimize empty LLVM module");
    target.emit_object(optimized).expect("emit verified object")
}

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
fn native_target_registration_should_be_safe_under_parallel_first_use() {
    let workers = (0..16)
        .map(|_| {
            std::thread::spawn(|| {
                for _ in 0..8 {
                    drop(NativeTarget::host().expect("create host target concurrently"));
                }
            })
        })
        .collect::<Vec<_>>();
    for worker in workers {
        worker.join().expect("native target worker must not panic");
    }
}

#[test]
fn native_module_and_object_should_create_and_drop_repeatedly() {
    for _ in 0..32 {
        let context = NativeContext::new().expect("create LLVM context");
        let target = NativeTarget::host().expect("create host target machine");
        let object = empty_object(&context, &target);
        assert!(!object.is_empty());
        drop(object);
        drop(target);
        drop(context);
    }
}

#[test]
fn injected_middle_stage_error_should_preserve_live_owner_relationships() {
    let context = NativeContext::new().expect("create LLVM context");
    let target = NativeTarget::host().expect("create host target machine");

    let injected = native_bridge_test_error();
    assert_eq!(injected.stage.to_string(), "LLVM bridge");

    let object = empty_object(&context, &target);
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
fn native_jit_should_use_the_selected_host_object_layer() {
    let jit = NativeJit::new().expect("create empty LLJIT");

    assert_eq!(jit.object_layer(), expected_host_object_layer());
}

#[test]
fn native_toolchain_should_own_context_target_and_jit() {
    let toolchain = NativeToolchain::new().expect("create native toolchain owners");

    assert_eq!(toolchain.object_layer(), expected_host_object_layer());
}
