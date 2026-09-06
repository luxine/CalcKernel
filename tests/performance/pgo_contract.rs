use std::{fs, process::Command};

use super::support::oracle::repo_root;

fn read(path: &str) -> String {
    fs::read_to_string(repo_root().join(path))
        .unwrap_or_else(|error| panic!("failed to read {path}: {error}"))
}

#[test]
fn schema_eight_harness_should_own_closed_workload_and_checker_assets() {
    let cargo = read("Cargo.toml");
    for required in [
        "name = \"pgo_perf\"",
        "path = \"benches/pgo_perf.rs\"",
        "harness = false",
    ] {
        assert!(
            cargo.contains(required),
            "missing benchmark contract {required}"
        );
    }

    let harness = read("benches/pgo_perf.rs");
    for required in [
        "cargo bench --features native-toolchain --bench pgo_perf",
        "scripts/measure-v013-performance.py",
        "--task",
        "collect",
        "--out",
    ] {
        assert!(
            harness.contains(required),
            "pgo harness must contain {required}"
        );
    }

    let manifest = read("benches/cases/pgo-cases.tsv");
    for required in [
        "ckc-pgo-cases\t1",
        "branch-layout",
        "call-constant-length",
        "trip-unroll-simd",
        "memory-bound",
        "compute-bound",
        "training",
        "held-out",
        "adversarial",
        "x86-64-v3",
        "x86-64-v4",
        "aarch64-sve",
        "aarch64-sve2",
    ] {
        assert!(
            manifest.contains(required),
            "workload manifest must contain {required}"
        );
    }

    for path in [
        "benches/fixtures/pgo/training.tsv",
        "benches/fixtures/pgo/held-out.tsv",
        "benches/fixtures/pgo/adversarial.tsv",
        "benches/fixtures/pgo/compute_bound.ck",
        "benches/oracles/pgo/c/pgo_oracle.c",
        "benches/oracles/pgo/rust/pgo_oracle.rs",
        "benches/oracles/pgo/manifest.toml",
    ] {
        assert!(
            repo_root().join(path).is_file(),
            "missing schema-8 asset {path}"
        );
    }
    let compute = read("benches/fixtures/pgo/compute_bound.ck");
    assert!(
        compute.matches("x = x *").count() >= 4,
        "the compute-bound corpus must contain a real arithmetic chain"
    );
    let measurement = read("scripts/measure-v013-performance.py");
    assert!(
        !measurement.contains("argtypes.append"),
        "ctypes signatures must be assigned atomically so every ABI argument is passed"
    );
    assert_eq!(
        measurement.matches("collect(args.out, args.quick)").count(),
        1,
        "one benchmark invocation must collect exactly one immutable evidence bundle"
    );
}

#[test]
fn schema_eight_checker_regressions_should_pass() {
    let output = Command::new("python3")
        .arg("-B")
        .arg(repo_root().join("tests/performance/pgo_gate_test.py"))
        .output()
        .expect("run schema-8 checker regression suite");
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn schema_eight_compile_time_should_measure_terminated_child_cpu_time() {
    let measurement = read("scripts/measure-v013-performance.py");
    let build_ck = &measurement[measurement.find("def build_ck(").expect("build_ck")
        ..measurement.find("\n\nclass Kernel:").expect("Kernel")];

    for required in [
        "def terminated_child_cpu_time_ns():",
        "resource.getrusage(resource.RUSAGE_CHILDREN)",
    ] {
        assert!(
            measurement.contains(required),
            "schema-8 compile-time measurement must use {required:?}"
        );
    }
    assert!(
        !build_ck.contains("time.perf_counter_ns()"),
        "schema-8 source-to-object samples must exclude hosted-runner descheduling time"
    );
}

#[test]
fn profile_generation_initializer_should_remain_out_of_instrumented_hot_paths() {
    let lowering = read("src/backend/llvm/kir_lower.rs");
    let builder = read("src/backend/llvm/builder.rs");
    let ffi = read("src/backend/llvm/ffi.rs");
    let bridge = read("native/bridge/ckc_llvm.cpp");

    assert!(
        lowering.contains("ensure.set_noinline()?;"),
        "the generated initialization wrapper must not be duplicated into hot functions"
    );
    for required in [
        "fn set_noinline",
        "function_set_noinline",
        "ckc_llvm_function_set_noinline",
        "llvm::Attribute::NoInline",
    ] {
        assert!(
            format!("{builder}\n{ffi}\n{bridge}").contains(required),
            "missing noinline bridge layer {required}"
        );
    }
}

#[test]
fn profile_generation_edges_should_batch_locally_until_function_exit() {
    let lowering = read("src/backend/llvm/kir_lower.rs");
    for required in [
        "edge_storage: BTreeMap<u32, Storage<'module>>",
        "fn add_profile_edge_local",
        "fn flush_profile_edge_counters",
    ] {
        assert!(
            lowering.contains(required),
            "profile edge batching is missing {required:?}"
        );
    }
    let edge_lowering = lowering
        .split("fn emit_profile_edge")
        .nth(1)
        .expect("profile edge lowering")
        .split("fn current_block")
        .next()
        .expect("profile edge lowering boundary");
    assert!(edge_lowering.contains("self.add_profile_edge_local"));
    assert!(
        !edge_lowering.contains("self.builder.call(increment"),
        "instrumented loop edges must not perform atomic increments"
    );
}

#[test]
fn profile_generation_candidates_should_batch_locally_until_function_exit() {
    let lowering = read("src/backend/llvm/kir_lower.rs");
    let runtime = read("native/profile_runtime/common/collector.c");
    let header = read("native/profile_runtime/include/ckc_profile_runtime.h");
    for required in [
        "candidate_storage: BTreeMap<u32, NativeProfileCandidateStorage<'module>>",
        "fn allocate_profile_candidate_counters",
        "fn add_profile_candidate_local",
        "fn flush_profile_candidate_counters",
        "__ck_profile_add_bucket",
    ] {
        assert!(
            format!("{lowering}\n{runtime}\n{header}").contains(required),
            "profile candidate batching is missing {required:?}"
        );
    }
    let instruction_lowering = lowering
        .split("fn emit_profile_instruction")
        .nth(1)
        .expect("profile instruction lowering")
        .split("fn instruction")
        .next()
        .expect("profile instruction lowering boundary");
    assert!(instruction_lowering.contains("self.add_profile_candidate_local"));
    assert!(
        !instruction_lowering.contains("builder.call(candidate_function"),
        "instrumented candidate sites must not call the atomic runtime in the hot path"
    );
}

#[test]
fn x86_checked_loops_should_request_bounded_llvm_unrolling() {
    let bridge = read("native/bridge/ckc_llvm.cpp");
    for required in [
        "attach_x86_checked_loop_unroll",
        "llvm.loop.unroll.count",
        "llvm::Intrinsic::uadd_with_overflow",
        "llvm::Triple::x86_64",
    ] {
        assert!(
            bridge.contains(required),
            "x86 checked-loop unroll handoff is missing {required:?}"
        );
    }
}

#[test]
fn aarch64_sve_loops_should_request_four_way_llvm_interleave() {
    let bridge = read("native/bridge/ckc_llvm.cpp");
    for required in [
        "constexpr uint32_t CKC_AARCH64_SVE_LOOP_INTERLEAVE = 4;",
        "attach_aarch64_sve_loop_interleave",
        "target.getTargetFeatureString().contains(\"+sve\")",
        "llvm.loop.interleave.count",
        "CKC_AARCH64_SVE_LOOP_INTERLEAVE",
    ] {
        assert!(
            bridge.contains(required),
            "AArch64 SVE loop interleave handoff is missing {required:?}"
        );
    }
}

#[test]
fn aarch64_sve_multiversion_should_use_a_fixed_schedule_without_expanding_isa() {
    let bridge = read("native/bridge/ckc_llvm.cpp");
    let commands = read("src/cli/commands.rs");
    let inline_policy = read("src/optimizer/mod.rs");
    let native_lowering = read("src/backend/llvm/kir_lower.rs");
    let contract = format!("{bridge}\n{commands}\n{inline_policy}\n{native_lowering}");
    for required in [
        "constexpr llvm::StringLiteral CKC_AARCH64_SVE_TUNE_CPU = \"neoverse-n2\";",
        "attach_aarch64_sve_tuning",
        "target.getTargetCPU() != \"generic\"",
        "target.getTargetFeatureString().contains(\"+sve\")",
        "function.addFnAttr(\"tune-cpu\", CKC_AARCH64_SVE_TUNE_CPU)",
        "function.addFnAttr(\"target-cpu\", target.getTargetCPU())",
        "function.addFnAttr(\"target-features\"",
        "aarch64-sve-tune-neoverse-n2-v2",
        "coverage-first-variant-ranking-v1",
        "coverage-companion-profitability-v1",
        "compact-multiversion-inline-v2",
        "x86-loop-simd-min-interleave-4-v1",
        "compact-vector-uf-stride-v1",
        "KIR_INLINE_CALLEE_BUDGET: usize = 32",
        "KIR_MULTIVERSION_INLINE_CALLEE_BUDGET: usize = 8",
        "KIR_PGO_HOT_INLINE_CALLEE_BUDGET: usize = 48",
        "compact_multiversion_noinline_functions",
        "handle.set_noinline()",
    ] {
        assert!(
            contract.contains(required),
            "fixed AArch64 SVE scheduling contract is missing {required:?}"
        );
    }
    assert!(
        !bridge.contains("function.addFnAttr(\"target-cpu\", CKC_AARCH64_SVE_TUNE_CPU)"),
        "the scheduling model must not imply additional ISA features"
    );
}

#[test]
fn native_handoff_repairs_should_preserve_contract_facts_and_constant_map_schedule() {
    let bridge = read("native/bridge/ckc_llvm.cpp");
    let multiversion = read("src/backend/llvm/multiversion.rs");
    let commands = read("src/cli/commands.rs");

    for required in [
        "CKC_X86_CONSTANT_MAP_INTERLEAVE = 1",
        "CKC_X86_CONSTANT_MAP_UNROLL = 5",
        "attach_x86_constant_call_map_schedule",
        "llvm.loop.interleave.count",
        "llvm.loop.unroll.count",
    ] {
        assert!(
            bridge.contains(required),
            "Native LLVM handoff must pin `{required}`"
        );
    }
    assert!(
        !bridge.contains("specialized_length"),
        "the bridge schedule must be selected from IR semantics, not a fixture name"
    );

    for required in ["contracts: &ContractFactSet", "Some(contracts)"] {
        assert!(
            multiversion.contains(required),
            "multiversion revalidation must preserve `{required}`"
        );
    }
    assert!(
        commands.contains("multiversion contract facts are missing"),
        "the CLI must fail closed instead of silently emitting fact-free variants"
    );
}

#[test]
fn schema_eight_docs_and_scripts_should_pin_exact_v013_contract() {
    let schema = read("benches/summary-schema.md");
    let checker = read("scripts/check-native-performance.py");
    let replay = read("scripts/prepare-performance-replay.py");
    let measurement = read("scripts/measure-v013-performance.py");
    let combined = format!("{schema}\n{checker}\n{replay}\n{measurement}");

    for required in [
        "schemaVersion: 8",
        "e1bcea461492a5a2619cdb960ea00dd668847f0a",
        "0.13.0",
        "22.1.8",
        "1.90.0",
        "rotating-eight-channel-v1",
        "candidateSha",
        "capabilityManifest",
        "trainingShards",
        "finalProfiles",
        "variantObjects",
        "selectedDirect",
        "dispatch_symbol_values",
        "bind_selected_direct",
        "resolverCalls",
        "cumulativeSchemaSeven",
        "archiveSize",
    ] {
        assert!(
            combined.contains(required),
            "schema-8 contract must contain {required}"
        );
    }

    for threshold in [
        "ordinaryGeoSlowdown=1.02",
        "ordinaryIndividualSlowdown=1.05",
        "pgoGeoImprovement=1.05",
        "pgoIndividualSlowdown=1.03",
        "dispatchGeoImprovement=1.08",
        "dispatchDirectGeoThroughput=0.98",
        "combinedGeoSlowdown=1.02",
        "oracleGeoThroughput=0.95",
        "generationOverhead=5.0",
        "archiveGrowth=1.15",
    ] {
        assert!(
            combined.contains(threshold),
            "schema-8 threshold must remain {threshold}"
        );
    }
}
