use std::sync::{
    Arc, Barrier,
    atomic::{AtomicUsize, Ordering},
};

use super::support::compiler::optimized_module;

use calckernel::{
    Aarch64AuxvSnapshot, BoundsMode, EmitLlvmOptions, KirConsumer, NATIVE_DISPATCH_RUNTIME_SHA256,
    NativeCapabilityCache, NativeCapabilitySet, NativeDispatchCandidate, NativeDispatchCell,
    NativeDispatchTable, NativeDispatchTier, NativeOptimizationLevel, NativeTarget, OverflowMode,
    X86CpuidSnapshot, detect_aarch64_auxv, detect_host_cpu_capabilities, detect_x86_cpuid,
    embedded_dispatch_runtime_object, lower_native_kir_module, test_add_multiversion_dispatch,
};

const TARGET_SET: [u8; 32] = [0x5a; 32];

fn table(ranked: &[NativeDispatchTier]) -> NativeDispatchTable {
    let candidates = ranked
        .iter()
        .enumerate()
        .map(|(index, tier)| NativeDispatchCandidate {
            tier: *tier,
            hidden_symbol: format!("implementation-{index}"),
            address: 0x1000 + index,
        })
        .collect::<Vec<_>>();
    NativeDispatchTable::new(TARGET_SET, "sum", candidates).expect("valid dispatch table")
}

#[test]
fn detector_x86_should_require_complete_hardware_and_os_state() {
    let v4 = X86CpuidSnapshot::complete_v4_fixture();
    assert_eq!(detect_x86_cpuid(v4), NativeCapabilitySet::X86_V4);

    let mut no_ymm = v4;
    no_ymm.xcr0 &= !X86CpuidSnapshot::XCR0_YMM;
    assert_eq!(detect_x86_cpuid(no_ymm), NativeCapabilitySet::BASELINE);

    let mut no_opmask = v4;
    no_opmask.xcr0 &= !X86CpuidSnapshot::XCR0_OPMASK;
    assert_eq!(detect_x86_cpuid(no_opmask), NativeCapabilitySet::X86_V3);

    let mut no_osxsave = v4;
    no_osxsave.leaf1_ecx &= !X86CpuidSnapshot::LEAF1_OSXSAVE;
    assert_eq!(detect_x86_cpuid(no_osxsave), NativeCapabilitySet::BASELINE);
}

#[test]
fn detector_linux_aarch64_should_require_initial_auxv_usable_state() {
    let sve2 = Aarch64AuxvSnapshot {
        query_succeeded: true,
        heterogeneous_uncertainty: false,
        hwcap: Aarch64AuxvSnapshot::HWCAP_SVE,
        hwcap2: Aarch64AuxvSnapshot::HWCAP2_SVE2,
        sve_state_usable: true,
        unknown_required_bits: false,
    };
    assert_eq!(detect_aarch64_auxv(sve2), NativeCapabilitySet::ARM_SVE2);

    let mut unusable = sve2;
    unusable.sve_state_usable = false;
    assert_eq!(detect_aarch64_auxv(unusable), NativeCapabilitySet::BASELINE);

    let contradictory = Aarch64AuxvSnapshot { hwcap: 0, ..sve2 };
    assert_eq!(
        detect_aarch64_auxv(contradictory),
        NativeCapabilitySet::BASELINE
    );
}

#[test]
fn detector_unknown_failure_and_heterogeneous_state_should_fail_to_baseline() {
    let mut x86 = X86CpuidSnapshot::complete_v4_fixture();
    x86.query_succeeded = false;
    assert_eq!(detect_x86_cpuid(x86), NativeCapabilitySet::BASELINE);
    x86.query_succeeded = true;
    x86.unknown_required_bits = true;
    assert_eq!(detect_x86_cpuid(x86), NativeCapabilitySet::BASELINE);
    x86.unknown_required_bits = false;
    x86.heterogeneous_uncertainty = true;
    assert_eq!(detect_x86_cpuid(x86), NativeCapabilitySet::BASELINE);
}

#[test]
fn multiversion_dispatch_should_follow_compiler_rank_not_numeric_tier_order() {
    let table = table(&[
        NativeDispatchTier::X86_64V3,
        NativeDispatchTier::X86_64V4,
        NativeDispatchTier::Baseline,
    ]);
    let selected = table
        .select(NativeCapabilitySet::X86_V4)
        .expect("compatible selection");
    assert_eq!(selected.tier, NativeDispatchTier::X86_64V3);
    assert_eq!(selected.address, 0x1000);
}

#[test]
fn multiversion_dispatch_should_be_baseline_stable_and_namespaced() {
    let table = table(&[
        NativeDispatchTier::AArch64Sve2,
        NativeDispatchTier::Baseline,
    ]);
    let selected = table
        .select(NativeCapabilitySet::BASELINE)
        .expect("baseline selection");
    assert_eq!(selected.tier, NativeDispatchTier::Baseline);
    assert!(table.public_symbol().starts_with("sum"));
    assert!(table.baseline_symbol().contains("5a5a5a5a5a5a5a5a"));
    assert!(table.support_symbol().contains("5a5a5a5a5a5a5a5a"));
    assert_ne!(table.public_symbol(), table.baseline_symbol());
}

#[test]
fn multiversion_dispatch_should_reject_malformed_or_unknown_tables() {
    let missing_baseline = NativeDispatchTable::new(
        TARGET_SET,
        "sum",
        vec![NativeDispatchCandidate {
            tier: NativeDispatchTier::X86_64V3,
            hidden_symbol: "v3".to_string(),
            address: 1,
        }],
    )
    .expect_err("baseline is mandatory");
    assert!(missing_baseline.contains("baseline"), "{missing_baseline}");

    let zero_pointer = NativeDispatchTable::new(
        TARGET_SET,
        "sum",
        vec![NativeDispatchCandidate {
            tier: NativeDispatchTier::Baseline,
            hidden_symbol: "baseline".to_string(),
            address: 0,
        }],
    )
    .expect_err("null implementation is invalid");
    assert!(zero_pointer.contains("pointer"), "{zero_pointer}");
}

#[test]
fn multiversion_dispatch_concurrent_first_call_should_query_once_and_publish_compatible_pointer() {
    let table = Arc::new(table(&[
        NativeDispatchTier::X86_64V4,
        NativeDispatchTier::X86_64V3,
        NativeDispatchTier::Baseline,
    ]));
    let cache = Arc::new(NativeCapabilityCache::new());
    let cell = Arc::new(NativeDispatchCell::new());
    let queries = Arc::new(AtomicUsize::new(0));
    let start = Arc::new(Barrier::new(17));
    let mut threads = Vec::new();
    for _ in 0..16 {
        let table = Arc::clone(&table);
        let cache = Arc::clone(&cache);
        let cell = Arc::clone(&cell);
        let queries = Arc::clone(&queries);
        let start = Arc::clone(&start);
        threads.push(std::thread::spawn(move || {
            start.wait();
            cell.resolve(&table, &cache, || {
                queries.fetch_add(1, Ordering::SeqCst);
                NativeCapabilitySet::X86_V3
            })
            .expect("resolve")
        }));
    }
    start.wait();
    for thread in threads {
        assert_eq!(thread.join().expect("join"), 0x1001);
    }
    assert_eq!(queries.load(Ordering::SeqCst), 1);
    assert_eq!(cache.initialization_count(), 1);
    assert_eq!(cell.slow_path_count(), 1);
    assert_eq!(cell.resolve_count(), 16);
}

#[test]
fn abi_multiversion_public_thunk_contract_should_preserve_public_address_and_hide_support() {
    let table = table(&[NativeDispatchTier::X86_64V3, NativeDispatchTier::Baseline]);
    let contract = table.thunk_contract("ck-native-abi-1:sum(slice<i32>,u32)->i32");
    assert_eq!(contract.public_symbol, "sum");
    assert_ne!(contract.public_symbol, contract.baseline_symbol);
    assert!(contract.baseline_symbol.starts_with("__ck_mv_"));
    assert!(contract.support_symbol.starts_with("__ck_mv_"));
    assert!(contract.hidden_symbols.iter().all(|name| name != "sum"));
    assert_eq!(
        contract.abi_signature,
        "ck-native-abi-1:sum(slice<i32>,u32)->i32"
    );
}

#[test]
fn differential_private_seam_should_only_force_compatible_verified_variants() {
    let table = table(&[
        NativeDispatchTier::X86_64V4,
        NativeDispatchTier::X86_64V3,
        NativeDispatchTier::Baseline,
    ]);
    assert_eq!(
        table
            .select_for_test(NativeCapabilitySet::X86_V3, NativeDispatchTier::X86_64V3)
            .expect("compatible forced tier")
            .address,
        0x1001
    );
    assert!(
        table
            .select_for_test(NativeCapabilitySet::X86_V3, NativeDispatchTier::X86_64V4)
            .is_err()
    );
}

#[test]
fn ownership_multiversion_dispatch_state_should_release_after_concurrent_use() {
    let table = Arc::new(table(&[
        NativeDispatchTier::X86_64V3,
        NativeDispatchTier::Baseline,
    ]));
    let cache = Arc::new(NativeCapabilityCache::new());
    let cell = Arc::new(NativeDispatchCell::new());
    let weak_table = Arc::downgrade(&table);
    let weak_cache = Arc::downgrade(&cache);
    let weak_cell = Arc::downgrade(&cell);
    let thread = {
        let table = Arc::clone(&table);
        let cache = Arc::clone(&cache);
        let cell = Arc::clone(&cell);
        std::thread::spawn(move || {
            cell.resolve(&table, &cache, || NativeCapabilitySet::X86_V3)
                .expect("resolve")
        })
    };
    assert_eq!(thread.join().expect("join"), 0x1000);
    drop(table);
    drop(cache);
    drop(cell);
    assert!(weak_table.upgrade().is_none());
    assert!(weak_cache.upgrade().is_none());
    assert!(weak_cell.upgrade().is_none());
}

#[test]
fn detector_real_host_query_should_return_a_normalized_closed_capability_set() {
    let capabilities = detect_host_cpu_capabilities();
    assert!(capabilities.is_normalized());
    assert!(matches!(
        capabilities,
        NativeCapabilitySet::BASELINE
            | NativeCapabilitySet::X86_V3
            | NativeCapabilitySet::X86_V4
            | NativeCapabilitySet::ARM_SVE
            | NativeCapabilitySet::ARM_SVE2
    ));
}

#[test]
fn multiversion_dispatch_runtime_object_should_be_independent_valid_and_content_addressed() {
    let bytes = embedded_dispatch_runtime_object();
    assert!(!bytes.is_empty());
    assert_eq!(NATIVE_DISPATCH_RUNTIME_SHA256.len(), 64);
    assert!(
        NATIVE_DISPATCH_RUNTIME_SHA256
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    );
    let object = NativeTarget::host_with_cpu(calckernel::NativeCpu::Baseline)
        .expect("baseline target")
        .parse_cached_object(bytes)
        .expect("validated dispatch runtime object");
    assert_eq!(object.as_bytes(), bytes);
}

#[test]
fn multiversion_dispatch_acceptance_artifact_set_should_be_auditable() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target/native-acceptance/v0.13-stage-08");
    super::artifacts::write_native_acceptance_artifact_set(&root);
    assert!(root.join("runtime/SHA256SUMS").is_file());
    assert!(
        root.join(if cfg!(target_os = "windows") {
            "module.dll"
        } else if cfg!(target_os = "macos") {
            "libmodule.dylib"
        } else {
            "libmodule.so"
        })
        .is_file()
    );
}

#[test]
fn abi_multiversion_llvm_thunk_should_keep_public_abi_and_publish_one_tail_target() {
    let kir = optimized_module(
        "export fn answer(value: i32) -> i32 { return value + 1; }",
        3,
        KirConsumer::NativeLibrary,
        OverflowMode::Unchecked,
        BoundsMode::Unchecked,
    );
    let context = calckernel::NativeContext::new().expect("context");
    let target =
        NativeTarget::host_with_cpu(calckernel::NativeCpu::Baseline).expect("baseline target");
    let ordinary = lower_native_kir_module(&context, &target, &kir, &EmitLlvmOptions::default())
        .expect("ordinary lowering")
        .verify()
        .expect("ordinary verify")
        .to_ir_string()
        .expect("ordinary IR");

    let mut dispatched =
        lower_native_kir_module(&context, &target, &kir, &EmitLlvmOptions::default())
            .expect("dispatch lowering");
    test_add_multiversion_dispatch(
        &mut dispatched,
        "answer",
        "__ck_impl_answer",
        "__ck_mv_answer_fixture_v3",
        NativeCapabilitySet::X86_V3.bits().into(),
    )
    .expect("install dispatcher");
    let verified = dispatched.verify().expect("dispatch verify");
    let text = verified.to_ir_string().expect("dispatch IR");
    assert_eq!(
        definition_signature(&ordinary, "answer"),
        definition_signature(&text, "answer")
    );
    assert!(
        text.contains("@__ck_mv_0102030405060708_answer_baseline"),
        "{text}"
    );
    assert!(text.contains("@__ck_mv_answer_fixture_v3"), "{text}");
    assert!(text.contains("load atomic ptr"), "{text}");
    assert!(text.contains("cmpxchg ptr"), "{text}");
    assert!(text.contains("musttail call"), "{text}");
    assert!(
        text.contains("@__ck_dispatch_detect_capabilities"),
        "{text}"
    );
    assert!(!text.contains("getenv"), "{text}");

    let optimized = verified
        .audit()
        .expect("dispatch fact audit")
        .optimize(&target, NativeOptimizationLevel::O3)
        .expect("baseline dispatch optimization");
    let optimized_ir = optimized.to_ir_string().expect("optimized dispatch IR");
    assert!(
        optimized_ir.contains("load atomic i64") || optimized_ir.contains("load atomic ptr"),
        "{optimized_ir}"
    );
    assert!(
        optimized_ir.contains("tail call") || optimized_ir.contains("musttail call"),
        "{optimized_ir}"
    );
    let object = target.emit_object(optimized).expect("dispatch object");
    assert!(!object.is_empty());
}

fn definition_signature<'a>(ir: &'a str, name: &str) -> &'a str {
    ir.lines()
        .find(|line| line.starts_with("define ") && line.contains(&format!("@{name}(")))
        .and_then(|line| line.split(" {").next())
        .expect("function definition")
}
