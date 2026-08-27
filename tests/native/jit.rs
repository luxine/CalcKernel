use calckernel::{BoundsMode, NativeJit, OrcObjectLayer, OverflowMode};

use super::runtime_support::executable_object;

#[test]
fn jit_should_eagerly_execute_the_same_o3_object_with_embedded_runtime_symbols() {
    let object = executable_object(
        "fn main() -> i32 { print_bool(true); print_newline(); return 42; }",
        OverflowMode::Unchecked,
        BoundsMode::Unchecked,
    );
    let mut jit = NativeJit::new().expect("create JIT");
    assert_eq!(jit.execute_entry(&object).expect("execute JIT entry"), 42);

    let error = jit
        .execute_entry(&object)
        .expect_err("one JIT instance must reject duplicate object definitions");
    assert_eq!(error.stage.to_string(), "LLVM ORC");
    assert!(error.message.contains("already executed"));
}

#[test]
fn jit_should_materialize_the_complete_object_graph_before_entry() {
    let object = executable_object(
        "fn main() -> i32 { return 0; }",
        OverflowMode::Unchecked,
        BoundsMode::Unchecked,
    );
    let mut jit = NativeJit::new().expect("create eager JIT");
    assert_eq!(jit.execute_entry(&object).expect("execute eager JIT"), 0);

    let audit = jit.memory_audit().expect("read eager JIT audit");
    assert!(
        audit.final_data_non_execute,
        "unused runtime data must already be materialized before entry: {audit:?}"
    );
}

#[test]
fn jit_should_execute_all_checked_mode_combinations_without_lazy_compilation() {
    for (overflow_mode, bounds_mode) in [
        (OverflowMode::Unchecked, BoundsMode::Unchecked),
        (OverflowMode::Checked, BoundsMode::Unchecked),
        (OverflowMode::Unchecked, BoundsMode::Checked),
        (OverflowMode::Checked, BoundsMode::Checked),
    ] {
        let object = executable_object(
            "fn main() -> i32 { return (20 + 1) * 2; }",
            overflow_mode,
            bounds_mode,
        );
        let mut jit = NativeJit::new().expect("create checked-mode JIT");
        assert_eq!(
            jit.execute_entry(&object)
                .expect("execute checked-mode JIT"),
            42,
            "{overflow_mode:?} {bounds_mode:?}"
        );
    }

    let bridge = include_str!("../../native/bridge/ckc_llvm.cpp");
    assert!(bridge.contains("addObjectFile"));
    for forbidden in [
        "CompileOnDemandLayer",
        "LazyCallThroughManager",
        "lazyReexports",
    ] {
        assert!(
            !bridge.contains(forbidden),
            "forbidden lazy ORC API {forbidden}"
        );
    }
}

#[test]
fn jit_object_layer_should_match_the_six_host_policy() {
    let actual = NativeJit::new().expect("create policy JIT").object_layer();
    let expected = if cfg!(all(target_os = "windows", target_arch = "aarch64")) {
        OrcObjectLayer::RuntimeDyldCoffAarch64
    } else {
        OrcObjectLayer::JitLink
    };
    assert_eq!(actual, expected);

    let bridge = include_str!("../../native/bridge/ckc_llvm.cpp");
    assert!(
        bridge.contains("CkcAuditedSectionMemoryManager"),
        "Windows AArch64 RuntimeDyld must use the audited memory manager"
    );
    assert!(
        bridge.contains("InvalidateInstructionCache"),
        "the RuntimeDyld path must explicitly finalize the instruction cache"
    );
}

#[test]
fn jit_memory_audit_should_prove_relocation_and_final_permissions() {
    let object = executable_object(
        "fn main() -> i32 { print_f64(1.5); print_newline(); return 0; }",
        OverflowMode::Unchecked,
        BoundsMode::Unchecked,
    );
    let mut jit = NativeJit::new().expect("create audited JIT");
    assert_eq!(jit.execute_entry(&object).expect("execute audited JIT"), 0);
    let audit = jit.memory_audit().expect("read JIT memory audit");

    assert!(audit.allocations > 0, "{audit:?}");
    assert!(audit.relocation_write_non_execute, "{audit:?}");
    assert!(audit.final_code_read_execute, "{audit:?}");
    assert!(audit.final_data_non_execute, "{audit:?}");
    assert!(audit.instruction_cache_finalizations > 0, "{audit:?}");
    #[cfg(target_os = "macos")]
    {
        assert!(audit.darwin_map_jit, "{audit:?}");
        assert!(audit.darwin_thread_write_protection, "{audit:?}");
    }
}
