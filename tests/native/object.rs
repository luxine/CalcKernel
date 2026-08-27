use calckernel::{
    EmitLlvmOptions, NativeContext, NativeCpu, NativeOptimizationLevel, NativeTarget, SourceFile,
    check, lower_native_llvm_module, lower_to_mir,
};

fn object_bytes(cpu: NativeCpu) -> Vec<u8> {
    let checked = check(&SourceFile::new(
        "object.ck",
        "export fn answer() -> i32 { return 42; }",
    ));
    assert_eq!(checked.diagnostics, []);
    let mir = lower_to_mir(&checked.checked_program).expect("lower MIR");
    let context = NativeContext::new().expect("native context");
    let target = NativeTarget::host_with_cpu(cpu).expect("host target");
    let optimized = lower_native_llvm_module(&context, &target, &mir, &EmitLlvmOptions::default())
        .expect("lower LLVM")
        .verify()
        .expect("verify LLVM")
        .optimize(&target, NativeOptimizationLevel::O3)
        .expect("optimize and reverify LLVM");
    target
        .emit_object(optimized)
        .expect("emit and parse object")
        .as_bytes()
        .to_vec()
}

#[test]
fn native_object_should_have_the_host_format_magic() {
    let bytes = object_bytes(NativeCpu::Baseline);
    assert!(bytes.len() > 64);
    if cfg!(target_os = "macos") {
        assert_eq!(&bytes[..4], &[0xcf, 0xfa, 0xed, 0xfe]);
    } else if cfg!(target_os = "windows") {
        assert!(
            matches!(&bytes[..2], [0x64, 0x86] | [0x64, 0xaa]),
            "COFF machine={:02x?}",
            &bytes[..2]
        );
    } else {
        assert_eq!(&bytes[..4], b"\x7fELF");
    }
}

#[test]
fn target_cpu_policy_should_distinguish_baseline_from_native() {
    let baseline = NativeTarget::host_with_cpu(NativeCpu::Baseline).expect("baseline target");
    let native = NativeTarget::host_with_cpu(NativeCpu::Native).expect("native target");
    let expected_baseline = match std::env::consts::ARCH {
        "x86_64" => "x86-64",
        "aarch64" => "generic",
        other => panic!("unsupported test architecture {other}"),
    };
    assert_eq!(baseline.cpu().expect("baseline CPU"), expected_baseline);
    assert_eq!(baseline.features().expect("baseline features"), "");
    assert!(!native.cpu().expect("native CPU").is_empty());
    let _complete_feature_selection = native.features().expect("native feature selection");
}
