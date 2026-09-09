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
fn schema_eight_runtime_channels_should_share_one_workspace() {
    let measurement = read("scripts/measure-v013-performance.py");
    let audit = read("scripts/audit-performance-oracles.py");

    assert!(
        measurement.contains("class KernelWorkspace:"),
        "schema-8 channels must separate mutable workload storage from loaded code"
    );
    assert!(
        measurement.contains("workspace = KernelWorkspace(case, held)")
            && measurement.contains("Kernel(artifacts[channel], case, workspace)"),
        "all timed channels must use the exact same input/output addresses"
    );
    assert!(
        measurement.contains("rotating-eight-channel-shared-workspace-v2"),
        "the changed sampling protocol must have a new replay identity"
    );
    assert!(
        audit.contains("workspace = measurement.KernelWorkspace(case, record)")
            && audit.contains("measurement.Kernel(library, case, workspace)"),
        "the PGO oracle audit must follow the shared-workspace Kernel constructor protocol"
    );
}

#[test]
fn multiversion_source_to_object_should_not_repeat_checked_frontend_work() {
    let planner = read("src/optimizer/multiversion.rs");
    let native = read("src/backend/llvm/multiversion.rs");
    let commands = read("src/cli/commands.rs");

    assert!(
        !planner.contains("print_kir_module"),
        "target-neutral body sharing must use structural KIR equality without serializing modules on the build path"
    );
    assert!(
        commands.contains("let checked_bundle = check_kir_multiversion_bundle"),
        "the CLI must retain the independent checker authority"
    );
    assert!(
        commands.contains("emit_native_multiversion_objects_checked"),
        "the CLI must pass the retained authority instead of reconstructing the proposal during emission"
    );
    assert!(
        commands.contains("&compiled.result")
            && native.contains("baseline_result: &KirPassManagerResult"),
        "native emission must reuse the already verified baseline result instead of running an O0 pipeline over it"
    );
    assert!(
        native.contains("pub fn emit_native_multiversion_objects(")
            && native.contains("check_kir_multiversion_bundle(request, bundle)"),
        "the public raw-bundle entry must remain fail closed"
    );
}

#[test]
fn multiversion_target_materialization_should_not_build_discarded_fixture_profiles() {
    let native = read("src/backend/llvm/multiversion.rs");
    let host = &native[native
        .find("impl NativeMultiversionTargetSet")
        .expect("native multiversion target implementation")
        ..native
            .find("fn error(message")
            .expect("native multiversion error boundary")];

    assert!(
        !host.contains("schema1_for_triple"),
        "Native target materialization must not build and discard complete synthetic cost profiles"
    );
}

#[test]
fn checked_multiversion_emission_should_not_repeat_variant_evidence_validation() {
    let pipeline = read("src/optimizer/kir_pipeline.rs");
    let native = read("src/backend/llvm/multiversion.rs");
    let lowering = read("src/backend/llvm/kir_lower.rs");

    for required in [
        "checked_multiversion_variant_result",
        "std::ptr::eq(candidate, variant)",
        "verification_cache: None",
    ] {
        assert!(
            pipeline.contains(required),
            "checked variant handoff omitted {required}"
        );
    }
    assert!(
        native.contains("checked_multiversion_variant_result(checked, variant, contracts)")
            && !native.contains(
                "run_kir_pass_pipeline(\n                variant.module.clone(),\n                KirOptimizationLevel::O0"
            ),
        "native emission must reuse the checked variant instead of repeating its O0 evidence validation"
    );
    assert!(
        lowering.contains("validate_kir_optimization_evidence(\n        kir,"),
        "LLVM lowering must retain the final fail-closed evidence validation"
    );
}

#[test]
fn multiversion_dispatch_hot_path_should_not_branch_on_a_null_slot() {
    let bridge = read("native/bridge/ckc_llvm.cpp");
    let commands = read("src/cli/commands.rs");
    assert!(
        bridge.contains("_resolve_entry"),
        "the dispatch slot must start at an ABI-preserving cold resolver entry"
    );
    assert!(
        !bridge.contains("ck.dispatch.uninitialized"),
        "the steady-state thunk must not retain a null test and branch"
    );
    assert!(
        commands.contains("dispatch-resolver-sentinel-v2"),
        "the object-affecting dispatcher change must invalidate Native caches"
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
fn profile_generation_internal_entries_should_batch_at_static_calls_until_caller_exit() {
    let lowering = read("src/backend/llvm/kir_lower.rs");
    let commands = read("src/cli/commands.rs");
    for required in [
        "callsite_entries: BTreeMap<String, u32>",
        "entry_storage: BTreeMap<u32, Storage<'module>>",
        "fn allocate_profile_entry_counters",
        "fn add_profile_entry_local",
        "fn flush_profile_entry_counters",
    ] {
        assert!(
            format!("{lowering}\n{commands}").contains(required),
            "profile entry call-site batching is missing {required:?}"
        );
    }
    let call_lowering = lowering
        .split("fn call(")
        .nth(1)
        .expect("call lowering")
        .split("fn terminator")
        .next()
        .expect("call lowering boundary");
    assert!(call_lowering.contains("self.add_profile_entry_local(name)?;"));
    let entry_lowering = lowering
        .split("fn emit_profile_function_entry")
        .nth(1)
        .expect("function-entry lowering")
        .split("fn emit_profile_instruction")
        .next()
        .expect("function-entry lowering boundary");
    assert!(entry_lowering.contains("self.function.exported || self.profile_entry"));
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
fn x86_checked_loops_should_use_a_memory_aware_bounded_schedule() {
    let bridge = read("native/bridge/ckc_llvm.cpp");
    let commands = read("src/cli/commands.rs");
    let checked_schedule = bridge
        .split("void attach_x86_checked_loop_unroll")
        .nth(1)
        .expect("x86 checked-loop schedule")
        .split("void attach_x86_integer_reduction_interleave")
        .next()
        .expect("x86 checked-loop schedule boundary");
    for required in [
        "CloneFunction(function, clone_map)",
        "promote_entry_allocas(*analysis)",
        "clone_map.lookup(loop->getHeader())",
        "checked_constant_bound_map",
        "argument_has_constant_equality_assume(*analysis, *bound)",
        "checked_constant_call_map",
        "scalar_memory_map_bound_argument(*analysis_loop)",
        "every_direct_call_has_constant_argument(*function, *bound)",
        "is_scalar_memory_map(*analysis_loop)",
        "llvm.loop.unroll.disable",
        "llvm.loop.unroll.count",
    ] {
        assert!(
            checked_schedule.contains(required),
            "x86 checked-loop unroll handoff is missing {required:?}"
        );
    }
    assert!(bridge.contains("bool argument_has_constant_equality_assume("));
    assert!(bridge.contains("llvm::Intrinsic::assume"));
    assert_eq!(
        commands
            .matches("x86-checked-memory-map-schedule-v3")
            .count(),
        2,
        "ordinary and multiversion Native object caches must bind the checked-map schedule"
    );
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
fn x86_v4_compute_loops_should_authorize_full_avx512_width() {
    let bridge = read("native/bridge/ckc_llvm.cpp");
    let commands = read("src/cli/commands.rs");
    for required in [
        "constexpr uint32_t CKC_X86_V4_F64_VECTOR_WIDTH = 8;",
        "attach_x86_v4_compute_loop_width",
        "target.getTargetCPU() != \"x86-64-v4\"",
        "is_compute_dense_strict_f64_map",
        "CKC_X86_V4_COMPUTE_MIN_F64_OPS = 8",
        "binary->getFastMathFlags().any()",
        "llvm.loop.vectorize.width",
        "llvm.loop.vectorize.enable",
        "CKC_X86_V4_F64_VECTOR_WIDTH",
        "x86-v4-compute-f64-width-8-v1",
    ] {
        assert!(
            bridge.contains(required) || commands.contains(required),
            "x86-64-v4 compute-loop width handoff is missing {required:?}"
        );
    }
}

#[test]
fn x86_v4_integer_maps_should_authorize_full_avx512_width() {
    let bridge = read("native/bridge/ckc_llvm.cpp");
    let commands = read("src/cli/commands.rs");
    for required in [
        "constexpr uint32_t CKC_X86_V4_I32_VECTOR_WIDTH = 16;",
        "is_wrapping_i32_memory_map",
        "load->isVolatile() || load->isAtomic()",
        "store->isVolatile() || store->isAtomic()",
        "CKC_X86_V4_I32_VECTOR_WIDTH",
        "x86-v4-i32-map-width-16-v1",
    ] {
        assert!(
            bridge.contains(required) || commands.contains(required),
            "x86-64-v4 integer-map width handoff is missing {required:?}"
        );
    }
}

#[test]
fn x86_v4_integer_maps_should_bound_scalar_tail_to_one_vector() {
    let bridge = read("native/bridge/ckc_llvm.cpp");
    let handoff = bridge
        .split("void attach_x86_v4_compute_loop_width")
        .nth(1)
        .expect("v4 schedule handoff")
        .split("void attach_x86_constant_call_map_schedule")
        .next()
        .expect("v4 schedule body");
    assert!(
        handoff.contains("llvm.loop.interleave.count")
            && handoff.contains("CKC_X86_V4_I32_INTERLEAVE"),
        "full-width integer maps must not multiply their scalar epilogue bound by LLVM's default interleave"
    );
    assert!(bridge.contains("constexpr uint32_t CKC_X86_V4_I32_INTERLEAVE = 1;"));
    assert!(read("src/cli/commands.rs").contains("x86-v4-i32-map-tail-width-16-v1"));
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
        "performance-first-dispatch-ranking-v1",
        "coverage-companion-profitability-v1",
        "shared-target-neutral-variant-budget-v1",
        "compact-multiversion-inline-v2",
        "x86-loop-simd-min-interleave-4-v1",
        "compact-vector-uf-stride-v1",
        "compact-vector-body-state-v2",
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
fn x86_widening_cast_frontend_budget_should_invalidate_every_native_object_cache() {
    let commands = read("src/cli/commands.rs");
    let identity = "x86-widening-cast-frontend-budget-2-v1";
    assert_eq!(
        commands.matches(identity).count(),
        2,
        "ordinary and multiversion native objects must both encode the widening-cast scheduling policy"
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
        "rotating-eight-channel-shared-workspace-v2",
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
