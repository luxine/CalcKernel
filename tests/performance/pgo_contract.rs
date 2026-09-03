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
fn schema_eight_docs_and_scripts_should_pin_exact_v013_contract() {
    let schema = read("benches/summary-schema.md");
    let checker = read("scripts/check-native-performance.py");
    let replay = read("scripts/prepare-performance-replay.py");
    let measurement = read("scripts/measure-v013-performance.py");
    let combined = format!("{schema}\n{checker}\n{replay}\n{measurement}");

    for required in [
        "schemaVersion: 8",
        "ea822e343967baa2db113d3dd8f429d8dfdfa779",
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
