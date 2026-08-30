use super::support::compiler::optimized_module;
use calckernel::{
    BoundsMode, EmitLlvmOptions, KirConsumer, NativeContext, NativeCpu, NativeOptimizationLevel,
    NativeTarget, OverflowMode, lower_native_kir_module,
};

fn object_bytes(cpu: NativeCpu) -> Vec<u8> {
    let kir = optimized_module(
        "export fn answer() -> i32 { return 42; }",
        3,
        KirConsumer::NativeLibrary,
        OverflowMode::Unchecked,
        BoundsMode::Unchecked,
    );
    let context = NativeContext::new().expect("native context");
    let target = NativeTarget::host_with_cpu(cpu).expect("host target");
    let optimized = lower_native_kir_module(&context, &target, &kir, &EmitLlvmOptions::default())
        .expect("lower LLVM")
        .verify()
        .expect("verify LLVM")
        .audit()
        .expect("audit LLVM facts")
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

#[cfg(target_os = "macos")]
#[test]
fn native_macho_calls_should_not_require_absolute_text_relocations() {
    let kir = optimized_module(
        "fn callee(n: i32) -> i32 { return n + 1; } export fn caller(n: i32) -> i32 { return callee(n); }",
        0,
        KirConsumer::NativeLibrary,
        OverflowMode::Unchecked,
        BoundsMode::Unchecked,
    );
    let context = NativeContext::new().expect("native context");
    let target = NativeTarget::host().expect("host target");
    let module = lower_native_kir_module(&context, &target, &kir, &EmitLlvmOptions::default())
        .expect("lower unoptimized calls")
        .verify()
        .expect("verify calls")
        .audit()
        .expect("audit calls")
        .optimize(&target, NativeOptimizationLevel::O0)
        .expect("retain unoptimized call boundary");
    let object = target.emit_object(module).expect("emit Mach-O calls");
    let bytes = object.as_bytes();
    let word = |offset: usize| {
        u32::from_le_bytes(bytes[offset..offset + 4].try_into().expect("Mach-O word"))
    };
    assert_eq!(word(0), 0xfeed_facf, "64-bit little-endian Mach-O");
    let mut command = 32;
    let mut text_relocations = 0;
    for _ in 0..word(16) {
        if word(command) == 0x19 {
            // LC_SEGMENT_64 contains 80-byte section_64 records after its header.
            for index in 0..word(command + 64) as usize {
                let section = command + 72 + index * 80;
                if &bytes[section..section + 7] != b"__text\0" {
                    continue;
                }
                let offset = word(section + 56) as usize;
                let count = word(section + 60) as usize;
                for relocation in 0..count {
                    let info = word(offset + relocation * 8 + 4);
                    // Both supported Mach-O architectures use type 0 for an
                    // absolute pointer. Internal calls must instead be PC-relative.
                    assert_ne!(info >> 28, 0, "absolute relocation in executable __text");
                }
                text_relocations += count;
            }
        }
        command += word(command + 4) as usize;
    }
    assert!(
        text_relocations > 0,
        "fixture must retain a real call relocation"
    );
}
