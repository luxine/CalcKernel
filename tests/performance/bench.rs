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
    const CLANG_CPU_DIGEST: &str =
        "f22d58f4e2712e792a5b933376fe3a81fa1bd44a4cdb39b2790359ab5a40c7f1";
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
    let clang_cpu_patch = fs::read(root.join("benches/baselines/v0_10_clang_cpu_harness.patch"))
        .expect("read v0.10 Clang CPU policy adapter");
    assert_eq!(
        format!("{:x}", Sha256::digest(&clang_cpu_patch)),
        CLANG_CPU_DIGEST
    );
    let clang_cpu_patch = String::from_utf8(clang_cpu_patch).expect("UTF-8 Clang CPU patch");
    for required in ["-march=x86-64", "-mtune=generic", "-mcpu=generic"] {
        assert!(
            clang_cpu_patch.contains(required),
            "Clang CPU adapter must contain {required:?}"
        );
    }

    let baseline = fs::read_to_string(root.join("benches/baselines/v0_10_compiler.toml"))
        .expect("read v0.10 baseline");
    assert!(
        baseline.starts_with("schema_version = 2\n"),
        "the paired Native/Clang baseline requires schema 2"
    );
    assert_eq!(
        baseline.matches("clang_median_ns = ").count(),
        24,
        "every frozen runtime entry must bind its same-run Clang oracle"
    );
    let frozen_c_oracles = [
        (
            "branch_mix-checked.c",
            "fb5b95130998c20a0014b01af5659720771d836614c5bd0aa85e5c02d68921e2",
        ),
        (
            "branch_mix-unchecked.c",
            "523e5f4af4c4bb64e6949dd7bfcd15578adb8ff47aa4437b5e1d01e6df84512b",
        ),
        (
            "integer_accumulate-checked.c",
            "91b9abc17ff50d7d55733ba0972f268779e8f2ea07ed96683dfa376a57113952",
        ),
        (
            "integer_accumulate-unchecked.c",
            "82b09a2e7428d99190cc50b03c709e5b018b082d0c265564bb4618e547fadf8a",
        ),
        (
            "proof_loop-checked.c",
            "044bc8d4b456a64d9cb6f3af057466796466b8cf32628fa4cb5e78b0e57bfee8",
        ),
        (
            "proof_loop-unchecked.c",
            "fed666f2048f254401e8554f8447b874cd4f602c1996f16825ea01d55e968326",
        ),
        (
            "remainder_chain-checked.c",
            "1dc89902f0e636a2c0a8f63a644a734ffcbbedb0b3039e299bc0c8b6ac439eda",
        ),
        (
            "remainder_chain-unchecked.c",
            "855c5bcb9bf82a8b06aab295c05211663a97a505654613a7b5dae33d2a6e9aeb",
        ),
    ];
    for (name, digest) in frozen_c_oracles {
        let bytes = fs::read(root.join("benches/baselines/v0_10_c_oracle").join(name))
            .expect("read frozen v0.10 C oracle");
        assert_eq!(format!("{:x}", Sha256::digest(bytes)), digest);
        assert!(
            baseline.contains(digest),
            "baseline identity must bind frozen C oracle {name}"
        );
    }
    assert!(
        baseline.contains(&format!("proof-loop ABI adapter sha256={PROOF_DIGEST}"))
            && baseline.contains(&format!("MIR optimizer timer sha256={OPTIMIZER_DIGEST}"))
            && baseline.contains(&format!(
                "Linux C++ runtime link adapter sha256={LINUX_CPP_RUNTIME_DIGEST}"
            ))
            && baseline.contains(&format!(
                "Clang CPU policy adapter sha256={CLANG_CPU_DIGEST}"
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
        "-march=x86-64",
        "-mtune=generic",
        "-mcpu=generic",
        "OverflowMode::Checked",
        "BoundsMode::Checked",
        "reference_equivalent",
        "native_cold_ns",
        "clang_c_cold_ns",
        "peak_memory_bytes",
        "artifact_bytes",
        "batch_iterations",
        "benches/baselines/v0_10_c_oracle/{name}-{suffix}.c",
        "v0_10_clang_median_ns",
    ] {
        assert!(
            harness.contains(required),
            "strict native runtime harness must mention `{required}`"
        );
    }
    let runtime_start = harness
        .find("fn measure_native_case(")
        .expect("native runtime case function");
    let runtime_end = harness[runtime_start..]
        .find("fn measure_kernel_call(")
        .map(|offset| runtime_start + offset)
        .expect("native runtime case boundary");
    assert!(
        !harness[runtime_start..runtime_end].contains("KirConsumer::C"),
        "runtime calibration must compile the frozen V0.10 C oracle, not candidate-emitted C"
    );
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
    let baseline_manifest = temp.join("baseline.toml");
    fs::write(
        &baseline_manifest,
        performance_baseline_manifest(&format!(
            "{}-{}",
            std::env::consts::OS,
            std::env::consts::ARCH
        )),
    )
    .expect("write synthetic frozen baseline manifest");

    let passing = temp.join("passing.json");
    fs::write(
        &passing,
        performance_report(100, 102, true, false, 100, 102, 150),
    )
    .expect("write passing report");
    let pass = Command::new("python3")
        .arg(&checker)
        .arg(&passing)
        .arg(&baseline_manifest)
        .output()
        .expect("run passing performance gate");
    assert!(
        pass.status.success(),
        "{}",
        String::from_utf8_lossy(&pass.stderr)
    );

    let common_mode_report = performance_report(100, 102, true, false, 100, 102, 150).replacen(
        &performance_case(
            "branch_mix",
            true,
            100,
            100,
            false,
            &performance_samples(100, 102),
        ),
        &performance_case_with_oracles(
            "branch_mix",
            true,
            120,
            120,
            100,
            100,
            false,
            &performance_samples(120, 122),
        ),
        1,
    );
    let common_mode_path = temp.join("common-mode.json");
    fs::write(&common_mode_path, common_mode_report).expect("write common-mode report");
    let common_mode = Command::new("python3")
        .arg(&checker)
        .arg(&common_mode_path)
        .arg(&baseline_manifest)
        .output()
        .expect("run common-mode performance gate");
    assert!(
        common_mode.status.success(),
        "common CK/Clang runner slowdown must be normalized against the frozen oracle: {}",
        String::from_utf8_lossy(&common_mode.stderr)
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
    let forged_baseline_clang = performance_report(100, 102, true, false, 100, 102, 150).replacen(
        r#""v010ClangMedianNs":100"#,
        r#""v010ClangMedianNs":99"#,
        1,
    );
    let full_samples = performance_samples(100, 102);
    let short_samples = performance_report(100, 102, true, false, 100, 102, 150).replacen(
        &format!(r#""nativeSamplesNs":{full_samples}"#),
        r#""nativeSamplesNs":[100,100,100]"#,
        1,
    );
    let omitted_runtime_case = performance_case(
        "remainder_chain",
        true,
        100,
        100,
        false,
        &performance_samples(100, 100),
    );
    let missing_runtime_case = performance_report(100, 102, true, false, 100, 102, 150)
        .replace(&format!(",{omitted_runtime_case}"), "");
    let full_optimizer = optimizer_comparisons(150);
    let missing_optimizer = full_optimizer.replace(
        r#",{"case":"example-dijkstra","kirMedianNs":150,"v010MirMedianNs":100}"#,
        "",
    );
    let missing_optimizer_case = performance_report(100, 102, true, false, 100, 102, 150)
        .replace(&full_optimizer, &missing_optimizer);

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
        ("v0-10-clang-oracle", forged_baseline_clang),
        ("sample-count", short_samples),
        ("runtime-corpus", missing_runtime_case),
        ("optimizer-corpus", missing_optimizer_case),
    ] {
        let path = temp.join(format!("{label}.json"));
        fs::write(&path, report).expect("write rejected report");
        let output = Command::new("python3")
            .arg(&checker)
            .arg(&path)
            .arg(&baseline_manifest)
            .output()
            .expect("run rejected performance gate");
        assert!(!output.status.success(), "{label} report must fail");
    }

    let optimizer_median = performance_report(100, 102, true, false, 100, 102, 150)
        .replace(
            &optimizer_comparisons(150),
            r#"[{"case":"pricing","kirMedianNs":10,"v010MirMedianNs":100},{"case":"pricing-soa","kirMedianNs":10,"v010MirMedianNs":100},{"case":"f64-kernels","kirMedianNs":210,"v010MirMedianNs":100},{"case":"proof","kirMedianNs":210,"v010MirMedianNs":100},{"case":"example-pricing","kirMedianNs":210,"v010MirMedianNs":100},{"case":"example-dijkstra","kirMedianNs":210,"v010MirMedianNs":100}]"#,
        );
    let path = temp.join("optimizer-median.json");
    fs::write(&path, optimizer_median).expect("write optimizer median rejection");
    let output = Command::new("python3")
        .arg(&checker)
        .arg(&path)
        .arg(&baseline_manifest)
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
    let native_samples = performance_samples(native_ns, last_native_sample);
    let proof_samples = performance_samples(100, 100);
    let checked_proof_samples = performance_samples(checked_proof_ns, checked_proof_ns);
    let branch_mix = performance_case(
        "branch_mix",
        equivalent,
        native_ns,
        baseline_ns,
        false,
        &native_samples,
    );
    let integer_accumulate = performance_case(
        "integer_accumulate",
        true,
        100,
        100,
        false,
        &performance_samples(100, 100),
    );
    let proof_loop = performance_case("proof_loop", true, 100, 100, true, &proof_samples);
    let checked_proof_loop = performance_case(
        "proof_loop",
        true,
        checked_proof_ns,
        100,
        true,
        &checked_proof_samples,
    );
    let remainder_chain = performance_case(
        "remainder_chain",
        true,
        100,
        100,
        false,
        &performance_samples(100, 100),
    );
    let optimizer_comparisons = optimizer_comparisons(kir_optimize_ns);
    format!(
        r#"{{
  "schemaVersion": 5,
  "cpuPolicy": "baseline",
  "fastMath": {fast_math},
  "clangVersion": "22.1.8",
  "warmup": 3,
  "sampleRepetitions": 7,
  "baselineV010": {{"commit":"df816502876fba41676f9ebc190e4fadd18cd5a5","compilerIdentity":"calckernel 0.10.0 (df816502876fba41676f9ebc190e4fadd18cd5a5)","llvmVersion":"22.1.8","target":"{target}","harness":"ckc_perf schema 2 + proof-loop ABI adapter sha256=316b64bf3e24ade271d870444bb66a85018c4dcb66229afce202da2d2b53af6e + MIR optimizer timer sha256=828138f376472b177d8bbd1aa4f7888ed323ec03d098e21a74abcfce32a98d0b + Linux C++ runtime link adapter sha256=099305e8a9d5ff8d54e574b0fbd202a511f28a8543508f8c0ea06001704cdaff + Clang CPU policy adapter sha256=f22d58f4e2712e792a5b933376fe3a81fa1bd44a4cdb39b2790359ab5a40c7f1; warmup=3; samples=20; repetitions=7; batch=20000000","statistics":"minimum-of-7 call samples; upper-median-of-20; strict-fp; pinned clang 22.1.8","sourceDigestCount":17,"sourceDigests":{{"branch_mix":"d4f80ba571422feffe4d568bd476b44dde2a3f9086d30ebd77972dcf4254d7b8","integer_accumulate":"4734807a96981f42e85b68ba4b964ce21e354c3486e7f668d89dcaefa391fc39","proof_loop":"ea8c9f1be3e5fffa8c1c0e5e448d6617be15d855fdef2ee49670c4f98b88e30d","remainder_chain":"87a36a9f5cd951c7281480bd180a9d8a657fd85e553f5a93edb2d5e74c00311e","pricing":"be74bd3851e54db09955255b463025a6ee8464620ae1753c88b7d6d453388416","pricing_soa":"5c003b70649f34516a2830584542086ce52ff0adfdd6dd0d76010a33e1d23cad","f64_kernels":"58e10d6c28c5d95088a2e156197eb51c880b361a555d016ae11e9e0b7ecad7be","example_pricing":"aebfe8bc5de317e32a7c945c7424a75b32a4330d7fd6dd53bb2d0c01cfbcb65a","example_dijkstra":"490a7a3a3a04abb9cb9f05c9dbeea60d61690fc32897f36916f1ffa3c28a2f96","v0_10_c_branch_mix_checked":"fb5b95130998c20a0014b01af5659720771d836614c5bd0aa85e5c02d68921e2","v0_10_c_branch_mix_unchecked":"523e5f4af4c4bb64e6949dd7bfcd15578adb8ff47aa4437b5e1d01e6df84512b","v0_10_c_integer_accumulate_checked":"91b9abc17ff50d7d55733ba0972f268779e8f2ea07ed96683dfa376a57113952","v0_10_c_integer_accumulate_unchecked":"82b09a2e7428d99190cc50b03c709e5b018b082d0c265564bb4618e547fadf8a","v0_10_c_proof_loop_checked":"044bc8d4b456a64d9cb6f3af057466796466b8cf32628fa4cb5e78b0e57bfee8","v0_10_c_proof_loop_unchecked":"fed666f2048f254401e8554f8447b874cd4f602c1996f16825ea01d55e968326","v0_10_c_remainder_chain_checked":"1dc89902f0e636a2c0a8f63a644a734ffcbbedb0b3039e299bc0c8b6ac439eda","v0_10_c_remainder_chain_unchecked":"855c5bcb9bf82a8b06aab295c05211663a97a505654613a7b5dae33d2a6e9aeb"}}}},
  "suites": [
    {{"mode":"unchecked","cases":[{branch_mix},{integer_accumulate},{proof_loop},{remainder_chain}]}},
    {{"mode":"checked","cases":[{branch_mix},{integer_accumulate},{checked_proof_loop},{remainder_chain}]}}
  ],
  "optimizerComparisons": {optimizer_comparisons}
}}"#
    )
}

fn performance_baseline_manifest(target: &str) -> String {
    let mut manifest = "schema_version = 2\ncommit = \"df816502876fba41676f9ebc190e4fadd18cd5a5\"\ncompiler_identity = \"calckernel 0.10.0 (df816502876fba41676f9ebc190e4fadd18cd5a5)\"\nllvm_version = \"22.1.8\"\n".to_string();
    for mode in ["unchecked", "checked"] {
        for case in [
            "branch_mix",
            "integer_accumulate",
            "proof_loop",
            "remainder_chain",
        ] {
            manifest.push_str(&format!(
                "\n[[runtime]]\ntarget = \"{target}\"\ncpu = \"baseline\"\nmode = \"{mode}\"\ncase = \"{case}\"\nmedian_ns = 100\nclang_median_ns = 100\n"
            ));
        }
    }
    manifest
}

fn performance_case(
    name: &str,
    equivalent: bool,
    native_ns: u64,
    baseline_ns: u64,
    proof_loop: bool,
    native_samples: &str,
) -> String {
    performance_case_with_oracles(
        name,
        equivalent,
        native_ns,
        100,
        baseline_ns,
        100,
        proof_loop,
        native_samples,
    )
}

#[allow(clippy::too_many_arguments)]
fn performance_case_with_oracles(
    name: &str,
    equivalent: bool,
    native_ns: u64,
    clang_ns: u64,
    baseline_ns: u64,
    baseline_clang_ns: u64,
    proof_loop: bool,
    native_samples: &str,
) -> String {
    let clang_samples = performance_samples(clang_ns, clang_ns);
    format!(
        r#"{{"name":"{name}","referenceEquivalent":{equivalent},"nativeMedianNs":{native_ns},"clangCMedianNs":{clang_ns},"v010MedianNs":{baseline_ns},"v010ClangMedianNs":{baseline_clang_ns},"proofLoop":{proof_loop},"nativeSamplesNs":{native_samples},"clangCSamplesNs":{clang_samples},"nativeCompileNs":100,"clangCCompileNs":100,"nativeColdNs":100,"clangCColdNs":100,"peakMemoryBytes":1024,"nativeArtifactBytes":1024,"clangCArtifactBytes":1024,"batchIterations":1000}}"#
    )
}

fn optimizer_comparisons(kir_optimize_ns: u64) -> String {
    let cases = [
        "pricing",
        "pricing-soa",
        "f64-kernels",
        "proof",
        "example-pricing",
        "example-dijkstra",
    ];
    format!(
        "[{}]",
        cases
            .iter()
            .map(|case| format!(
                r#"{{"case":"{case}","kirMedianNs":{kir_optimize_ns},"v010MirMedianNs":100}}"#
            ))
            .collect::<Vec<_>>()
            .join(",")
    )
}

fn performance_samples(primary: u64, alternate: u64) -> String {
    let mut samples = [primary; 20];
    samples[15..].fill(alternate);
    format!(
        "[{}]",
        samples
            .iter()
            .map(u64::to_string)
            .collect::<Vec<_>>()
            .join(",")
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
