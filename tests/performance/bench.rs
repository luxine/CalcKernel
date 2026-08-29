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
fn v0_10_baseline_harness_adapters_should_be_checksum_pinned() {
    const PROOF_DIGEST: &str = "316b64bf3e24ade271d870444bb66a85018c4dcb66229afce202da2d2b53af6e";
    const OPTIMIZER_DIGEST: &str =
        "828138f376472b177d8bbd1aa4f7888ed323ec03d098e21a74abcfce32a98d0b";
    const LINUX_CPP_RUNTIME_DIGEST: &str =
        "099305e8a9d5ff8d54e574b0fbd202a511f28a8543508f8c0ea06001704cdaff";
    let root = repo_root();
    let patch = fs::read(root.join("benches/baselines/v0_10_proof_loop_harness.patch"))
        .expect("read v0.10 proof-loop harness adapter");
    assert_eq!(format!("{:x}", Sha256::digest(&patch)), PROOF_DIGEST);
    let optimizer_patch =
        fs::read(root.join("benches/baselines/v0_10_mir_optimizer_harness.patch"))
            .expect("read v0.10 MIR optimizer harness adapter");
    assert_eq!(
        format!("{:x}", Sha256::digest(&optimizer_patch)),
        OPTIMIZER_DIGEST
    );
    let optimizer_patch = String::from_utf8(optimizer_patch).expect("UTF-8 optimizer patch");
    for required in [
        "let start = Instant::now();",
        "run_mir_pass_pipeline(mir, &pipeline, &context)",
        "v0-10-mir-optimizer.tsv",
    ] {
        assert!(
            optimizer_patch.contains(required),
            "optimizer adapter must contain {required:?}"
        );
    }
    let preparation = optimizer_patch
        .find("let mir = lower_to_mir")
        .expect("MIR preparation");
    let timer = optimizer_patch
        .find("let start = Instant::now()")
        .expect("optimizer timer");
    let pipeline = optimizer_patch
        .find("run_mir_pass_pipeline(mir, &pipeline, &context)")
        .expect("MIR pass pipeline");
    assert!(
        preparation < timer && timer < pipeline,
        "MIR construction must remain outside the optimizer timing region"
    );
    let linux_cpp_runtime_patch =
        fs::read(root.join("benches/baselines/v0_10_linux_cpp_runtime_harness.patch"))
            .expect("read v0.10 Linux C++ runtime link adapter");
    assert_eq!(
        format!("{:x}", Sha256::digest(&linux_cpp_runtime_patch)),
        LINUX_CPP_RUNTIME_DIGEST
    );
    let linux_cpp_runtime_patch =
        String::from_utf8(linux_cpp_runtime_patch).expect("UTF-8 Linux link patch");
    for required in [
        "-print-file-name=libstdc++.a",
        "cargo::rustc-link-search=native=",
        "cargo::rustc-link-lib=static=stdc++",
    ] {
        assert!(
            linux_cpp_runtime_patch.contains(required),
            "Linux link adapter must contain {required:?}"
        );
    }

    let baseline = fs::read_to_string(root.join("benches/baselines/v0_10_compiler.toml"))
        .expect("read v0.10 baseline");
    assert!(
        baseline.contains(&format!("proof-loop ABI adapter sha256={PROOF_DIGEST}"))
            && baseline.contains(&format!("MIR optimizer timer sha256={OPTIMIZER_DIGEST}"))
            && baseline.contains(&format!(
                "Linux C++ runtime link adapter sha256={LINUX_CPP_RUNTIME_DIGEST}"
            )),
        "baseline harness identity must bind every adapter"
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

    let forged_identity = performance_report(100, 102, true, false, 100, 102, 150).replace(
        "calckernel 0.10.0 (df816502876fba41676f9ebc190e4fadd18cd5a5)",
        "calckernel 0.10.0 (forged)",
    );
    let forged_digest = performance_report(100, 102, true, false, 100, 102, 150).replace(
        "d4f80ba571422feffe4d568bd476b44dde2a3f9086d30ebd77972dcf4254d7b8",
        "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
    );
    let wrong_clang = performance_report(100, 102, true, false, 100, 102, 150)
        .replace(r#""clangVersion": "22.1.8""#, r#""clangVersion": "23.0.0""#);
    let native_cpu = performance_report(100, 102, true, false, 100, 102, 150)
        .replace(r#""cpuPolicy": "baseline""#, r#""cpuPolicy": "native""#);
    let false_median = performance_report(100, 102, true, false, 100, 102, 150).replacen(
        r#""nativeMedianNs":100"#,
        r#""nativeMedianNs":1"#,
        1,
    );
    let wrong_warmup = performance_report(100, 102, true, false, 100, 102, 150)
        .replace(r#""warmup": 3"#, r#""warmup": 2"#);
    let wrong_repetitions = performance_report(100, 102, true, false, 100, 102, 150)
        .replace(r#""sampleRepetitions": 7"#, r#""sampleRepetitions": 3"#);

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
        ("baseline-identity", forged_identity),
        ("source-digest", forged_digest),
        ("clang-version", wrong_clang),
        ("native-cpu-policy", native_cpu),
        ("reported-median", false_median),
        ("warmup-identity", wrong_warmup),
        ("repetition-identity", wrong_repetitions),
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

    let optimizer_median = performance_report(100, 102, true, false, 100, 102, 150)
        .replace(
            r#""optimizerComparisons": [{"case":"pricing","kirMedianNs":150,"v010MirMedianNs":100}]"#,
            r#""optimizerComparisons": [{"case":"a","kirMedianNs":10,"v010MirMedianNs":100},{"case":"b","kirMedianNs":10,"v010MirMedianNs":100},{"case":"c","kirMedianNs":210,"v010MirMedianNs":100},{"case":"d","kirMedianNs":210,"v010MirMedianNs":100},{"case":"e","kirMedianNs":210,"v010MirMedianNs":100},{"case":"f","kirMedianNs":210,"v010MirMedianNs":100}]"#,
        );
    let path = temp.join("optimizer-median.json");
    fs::write(&path, optimizer_median).expect("write optimizer median rejection");
    let output = Command::new("python3")
        .arg(&checker)
        .arg(&path)
        .output()
        .expect("run optimizer median rejection");
    assert!(
        !output.status.success(),
        "optimizer suite median above 2x must fail even when its geometric mean passes"
    );
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
    let target = format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH);
    format!(
        r#"{{
  "schemaVersion": 4,
  "cpuPolicy": "baseline",
  "fastMath": {fast_math},
  "clangVersion": "22.1.8",
  "warmup": 3,
  "sampleRepetitions": 7,
  "baselineV010": {{"commit":"df816502876fba41676f9ebc190e4fadd18cd5a5","compilerIdentity":"calckernel 0.10.0 (df816502876fba41676f9ebc190e4fadd18cd5a5)","llvmVersion":"22.1.8","target":"{target}","harness":"ckc_perf schema 2 + proof-loop ABI adapter sha256=316b64bf3e24ade271d870444bb66a85018c4dcb66229afce202da2d2b53af6e + MIR optimizer timer sha256=828138f376472b177d8bbd1aa4f7888ed323ec03d098e21a74abcfce32a98d0b + Linux C++ runtime link adapter sha256=099305e8a9d5ff8d54e574b0fbd202a511f28a8543508f8c0ea06001704cdaff; warmup=3; samples=20; repetitions=7; batch=20000000","statistics":"minimum-of-7 call samples; upper-median-of-20; strict-fp; pinned clang 22.1.8","sourceDigestCount":9,"sourceDigests":{{"branch_mix":"d4f80ba571422feffe4d568bd476b44dde2a3f9086d30ebd77972dcf4254d7b8","integer_accumulate":"4734807a96981f42e85b68ba4b964ce21e354c3486e7f668d89dcaefa391fc39","proof_loop":"ea8c9f1be3e5fffa8c1c0e5e448d6617be15d855fdef2ee49670c4f98b88e30d","remainder_chain":"87a36a9f5cd951c7281480bd180a9d8a657fd85e553f5a93edb2d5e74c00311e","pricing":"be74bd3851e54db09955255b463025a6ee8464620ae1753c88b7d6d453388416","pricing_soa":"5c003b70649f34516a2830584542086ce52ff0adfdd6dd0d76010a33e1d23cad","f64_kernels":"58e10d6c28c5d95088a2e156197eb51c880b361a555d016ae11e9e0b7ecad7be","example_pricing":"aebfe8bc5de317e32a7c945c7424a75b32a4330d7fd6dd53bb2d0c01cfbcb65a","example_dijkstra":"490a7a3a3a04abb9cb9f05c9dbeea60d61690fc32897f36916f1ffa3c28a2f96"}}}},
  "suites": [
    {{"mode":"unchecked","cases":[{{"name":"integer","referenceEquivalent":{equivalent},"nativeMedianNs":{native_ns},"clangCMedianNs":100,"v010MedianNs":{baseline_ns},"proofLoop":false,"nativeSamplesNs":[{native_ns},{native_ns},{last_native_sample}],"clangCSamplesNs":[99,100,101],"nativeCompileNs":100,"clangCCompileNs":100,"nativeColdNs":100,"clangCColdNs":100,"peakMemoryBytes":1024,"nativeArtifactBytes":1024,"clangCArtifactBytes":1024,"batchIterations":1000}},{{"name":"proof_loop","referenceEquivalent":true,"nativeMedianNs":100,"clangCMedianNs":100,"v010MedianNs":100,"proofLoop":true,"nativeSamplesNs":[99,100,101],"clangCSamplesNs":[99,100,101],"nativeCompileNs":100,"clangCCompileNs":100,"nativeColdNs":100,"clangCColdNs":100,"peakMemoryBytes":1024,"nativeArtifactBytes":1024,"clangCArtifactBytes":1024,"batchIterations":1000}}]}},
    {{"mode":"checked","cases":[{{"name":"integer","referenceEquivalent":{equivalent},"nativeMedianNs":{native_ns},"clangCMedianNs":100,"v010MedianNs":{baseline_ns},"proofLoop":false,"nativeSamplesNs":[{native_ns},{native_ns},{last_native_sample}],"clangCSamplesNs":[99,100,101],"nativeCompileNs":100,"clangCCompileNs":100,"nativeColdNs":100,"clangCColdNs":100,"peakMemoryBytes":1024,"nativeArtifactBytes":1024,"clangCArtifactBytes":1024,"batchIterations":1000}},{{"name":"proof_loop","referenceEquivalent":true,"nativeMedianNs":{checked_proof_ns},"clangCMedianNs":100,"v010MedianNs":{checked_proof_ns},"proofLoop":true,"nativeSamplesNs":[{checked_proof_ns},{checked_proof_ns},{checked_proof_ns}],"clangCSamplesNs":[99,100,101],"nativeCompileNs":100,"clangCCompileNs":100,"nativeColdNs":100,"clangCColdNs":100,"peakMemoryBytes":1024,"nativeArtifactBytes":1024,"clangCArtifactBytes":1024,"batchIterations":1000}}]}}
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
