use std::{fs, process::Command};

use sha2::{Digest, Sha256};

use super::support::oracle::repo_root;

#[test]
fn repository_should_define_native_cargo_benchmark_harness() {
    let cargo_toml = fs::read_to_string(repo_root().join("Cargo.toml")).expect("read Cargo.toml");

    for required in [
        "[[bench]]",
        "name = \"ckc_perf\"",
        "path = \"benches/ckc_perf.rs\"",
        "harness = false",
    ] {
        assert!(
            cargo_toml.contains(required),
            "Cargo.toml must register a native ckc_perf benchmark with `{required}`"
        );
    }

    assert!(
        repo_root().join("benches/ckc_perf.rs").is_file(),
        "benches/ckc_perf.rs must contain the native performance harness"
    );
}

#[test]
fn benchmark_tree_should_own_final_assets() {
    for relative in ["benches/baselines", "benches/cases", "benches/fixtures"] {
        let dir = repo_root().join(relative);
        assert!(dir.is_dir(), "{relative} must exist");
        assert!(
            fs::read_dir(&dir)
                .expect("read benchmark directory")
                .next()
                .is_some(),
            "{relative} must contain benchmark-owned files"
        );
    }
    assert!(repo_root().join("benches/summary-schema.md").is_file());
    assert!(!repo_root().join("bench").exists());
}

#[test]
fn v0_10_proof_loop_harness_adapter_should_be_checksum_pinned() {
    const DIGEST: &str = "316b64bf3e24ade271d870444bb66a85018c4dcb66229afce202da2d2b53af6e";
    let root = repo_root();
    let patch = fs::read(root.join("benches/baselines/v0_10_proof_loop_harness.patch"))
        .expect("read v0.10 proof-loop harness adapter");
    assert_eq!(format!("{:x}", Sha256::digest(&patch)), DIGEST);

    let baseline = fs::read_to_string(root.join("benches/baselines/v0_10_compiler.toml"))
        .expect("read v0.10 baseline");
    assert!(
        baseline.contains(&format!("proof-loop ABI adapter sha256={DIGEST}")),
        "baseline harness identity must bind the proof-loop adapter"
    );
}

#[test]
fn benchmark_harness_should_cover_compiler_stages_and_backends() {
    let harness = fs::read_to_string(repo_root().join("benches/ckc_perf.rs"))
        .expect("read benchmark harness");

    for required in [
        "cargo bench --bench ckc_perf",
        "benches/fixtures",
        "emit_c_kir_module_with_contracts",
        "emit_wat_kir_module",
        "emit_wasm_kir_module",
        "EmitWasmOptions { opt_level: 3 }",
        "lower_native_kir_module",
        "run_kir_pass_pipeline",
        "v0_10_compiler.toml",
        "build/perf/latest.summary.json",
        "build/perf/latest.summary.md",
    ] {
        assert!(
            harness.contains(required),
            "native benchmark harness must mention `{required}`"
        );
    }
}

#[test]
fn native_runtime_harness_should_define_strict_equivalent_differential_measurement() {
    let root = repo_root();
    let harness =
        fs::read_to_string(root.join("benches/ckc_perf.rs")).expect("read benchmark harness");
    for required in [
        "tests/fixtures/performance/native",
        "CKC_CLANG_ORACLE",
        "NativeCpu::Baseline",
        "NativeCpu::Native",
        "-fno-fast-math",
        "-ffp-contract=off",
        "-fno-unwind-tables",
        "-fno-asynchronous-unwind-tables",
        "-falign-functions=64",
        "OverflowMode::Checked",
        "BoundsMode::Checked",
        "reference_equivalent",
        "native_cold_ns",
        "clang_c_cold_ns",
        "peak_memory_bytes",
        "artifact_bytes",
        "batch_iterations",
    ] {
        assert!(
            harness.contains(required),
            "strict native runtime harness must mention `{required}`"
        );
    }
    let bridge =
        fs::read_to_string(root.join("native/bridge/ckc_llvm.cpp")).expect("read native bridge");
    assert!(
        bridge.contains("function->setAlignment(llvm::Align(64))"),
        "native and strict C reference exports must share function alignment"
    );

    let fixture_root = root.join("tests/fixtures/performance/native");
    let fixtures = fs::read_dir(&fixture_root)
        .expect("read native performance fixtures")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect native performance fixtures");
    assert!(
        fixtures.len() >= 3,
        "runtime suite needs at least three kernels"
    );
    assert!(
        fixtures.iter().all(|entry| {
            entry.path().extension().and_then(|value| value.to_str()) == Some("ck")
        })
    );
}

#[test]
fn native_performance_gate_should_enforce_equivalence_stability_and_thresholds() {
    let root = repo_root();
    let checker = root.join("scripts/check-native-performance.py");
    assert!(checker.is_file(), "native performance checker must exist");
    let temp = std::env::temp_dir().join(format!(
        "ckc-performance-gate-{}-{}",
        std::process::id(),
        super::support::temp::unique_id()
    ));
    fs::create_dir_all(&temp).expect("create performance gate fixture");

    let passing = temp.join("passing.json");
    fs::write(
        &passing,
        performance_report(100, 102, true, false, 100, 102, 150),
    )
    .expect("write passing report");
    let pass = Command::new("python3")
        .arg(&checker)
        .arg(&passing)
        .output()
        .expect("run passing performance gate");
    assert!(
        pass.status.success(),
        "{}",
        String::from_utf8_lossy(&pass.stderr)
    );

    for (label, report) in [
        (
            "native-clang",
            performance_report(112, 100, true, false, 112, 102, 150),
        ),
        (
            "v0-10-baseline",
            performance_report(100, 102, true, false, 80, 102, 150),
        ),
        (
            "proof-loop",
            performance_report(100, 102, true, false, 100, 110, 150),
        ),
        (
            "optimizer",
            performance_report(100, 102, true, false, 100, 102, 350),
        ),
        (
            "equivalence",
            performance_report(100, 102, false, false, 100, 102, 150),
        ),
        (
            "fast-math",
            performance_report(100, 102, true, true, 100, 102, 150),
        ),
        (
            "stability",
            performance_report(100, 180, true, false, 100, 102, 150),
        ),
    ] {
        let path = temp.join(format!("{label}.json"));
        fs::write(&path, report).expect("write rejected report");
        let output = Command::new("python3")
            .arg(&checker)
            .arg(&path)
            .output()
            .expect("run rejected performance gate");
        assert!(!output.status.success(), "{label} report must fail");
    }
}

fn performance_report(
    native_ns: u64,
    last_native_sample: u64,
    equivalent: bool,
    fast_math: bool,
    baseline_ns: u64,
    checked_proof_ns: u64,
    kir_optimize_ns: u64,
) -> String {
    format!(
        r#"{{
  "schemaVersion": 4,
  "cpuPolicy": "baseline",
  "fastMath": {fast_math},
  "warmup": 3,
  "sampleRepetitions": 3,
  "baselineV010": {{"commit":"df816502876fba41676f9ebc190e4fadd18cd5a5","compilerIdentity":"calckernel 0.10.0","llvmVersion":"22.1.8","target":"test","harness":"test","statistics":"median","sourceDigestCount":9,"sourceDigests":{{"branch_mix":"0000000000000000000000000000000000000000000000000000000000000000","integer_accumulate":"0000000000000000000000000000000000000000000000000000000000000000","proof_loop":"0000000000000000000000000000000000000000000000000000000000000000","remainder_chain":"0000000000000000000000000000000000000000000000000000000000000000","pricing":"0000000000000000000000000000000000000000000000000000000000000000","pricing_soa":"0000000000000000000000000000000000000000000000000000000000000000","f64_kernels":"0000000000000000000000000000000000000000000000000000000000000000","example_pricing":"0000000000000000000000000000000000000000000000000000000000000000","example_dijkstra":"0000000000000000000000000000000000000000000000000000000000000000"}}}},
  "suites": [
    {{"mode":"unchecked","cases":[{{"name":"integer","referenceEquivalent":{equivalent},"nativeMedianNs":{native_ns},"clangCMedianNs":100,"v010MedianNs":{baseline_ns},"proofLoop":false,"nativeSamplesNs":[100,101,{last_native_sample}],"clangCSamplesNs":[99,100,101],"nativeCompileNs":100,"clangCCompileNs":100,"nativeColdNs":100,"clangCColdNs":100,"peakMemoryBytes":1024,"nativeArtifactBytes":1024,"clangCArtifactBytes":1024,"batchIterations":1000}},{{"name":"proof_loop","referenceEquivalent":true,"nativeMedianNs":100,"clangCMedianNs":100,"v010MedianNs":100,"proofLoop":true,"nativeSamplesNs":[99,100,101],"clangCSamplesNs":[99,100,101],"nativeCompileNs":100,"clangCCompileNs":100,"nativeColdNs":100,"clangCColdNs":100,"peakMemoryBytes":1024,"nativeArtifactBytes":1024,"clangCArtifactBytes":1024,"batchIterations":1000}}]}},
    {{"mode":"checked","cases":[{{"name":"integer","referenceEquivalent":{equivalent},"nativeMedianNs":{native_ns},"clangCMedianNs":100,"v010MedianNs":{baseline_ns},"proofLoop":false,"nativeSamplesNs":[100,101,{last_native_sample}],"clangCSamplesNs":[99,100,101],"nativeCompileNs":100,"clangCCompileNs":100,"nativeColdNs":100,"clangCColdNs":100,"peakMemoryBytes":1024,"nativeArtifactBytes":1024,"clangCArtifactBytes":1024,"batchIterations":1000}},{{"name":"proof_loop","referenceEquivalent":true,"nativeMedianNs":{checked_proof_ns},"clangCMedianNs":100,"v010MedianNs":{checked_proof_ns},"proofLoop":true,"nativeSamplesNs":[{checked_proof_ns},{checked_proof_ns},{checked_proof_ns}],"clangCSamplesNs":[99,100,101],"nativeCompileNs":100,"clangCCompileNs":100,"nativeColdNs":100,"clangCColdNs":100,"peakMemoryBytes":1024,"nativeArtifactBytes":1024,"clangCArtifactBytes":1024,"batchIterations":1000}}]}}
  ],
  "optimizerComparisons": [{{"case":"pricing","kirMedianNs":{kir_optimize_ns},"v010MirMedianNs":100}}]
}}"#
    )
}

#[test]
fn benchmark_docs_should_explain_native_cargo_bench_workflow() {
    let docs = [
        fs::read_to_string(repo_root().join("docs/guides/performance.md"))
            .expect("read performance doc"),
        fs::read_to_string(repo_root().join("docs/zh-CN/guides/performance.md"))
            .expect("read zh performance doc"),
    ];

    for text in docs {
        for required in [
            "cargo bench --bench ckc_perf",
            "build/perf/latest.summary.json",
            "build/perf/latest.summary.md",
        ] {
            assert!(
                text.contains(required),
                "benchmark docs must describe `{required}`"
            );
        }
    }
}
