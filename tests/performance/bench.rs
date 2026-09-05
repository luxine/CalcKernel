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
fn native_runtime_harness_should_prepare_validate_interleave_and_retain_replay_evidence() {
    let harness = fs::read_to_string(repo_root().join("benches/ckc_perf.rs")).unwrap();
    for required in [
        "CKC_V011_RUNTIME_BUNDLE",
        "CKC_V010_RUNTIME_BUNDLE",
        "load_replay(",
        "prepare_native_case(",
        "sample_channels(",
        "replay_v011_native_samples_ns",
        "replay_v011_clang_samples_ns",
        "replay_v010_native_samples_ns",
        "replay_v010_clang_samples_ns",
        "warmup_order",
        "sample_order",
        "measured_artifacts",
        "evidence_directory",
    ] {
        assert!(
            harness.contains(required),
            "missing actual replay integration: {required}"
        );
    }
    let runtime = &harness[harness.find("fn measure_native_runtime(").unwrap()
        ..harness.find("fn measure_kernel_call(").unwrap()];
    assert!(
        !runtime.contains("remove_dir_all"),
        "retain measured artifacts even on failure"
    );
    assert!(
        !runtime.contains("KirConsumer::C"),
        "both C copies must use only frozen sources"
    );
    assert!(runtime.find("load_replay(").unwrap() < runtime.find("measure_native_case(").unwrap());
}

#[test]
fn schema_seven_harness_should_measure_vector_domain_size_and_compile_time_corpora() {
    let root = repo_root();
    let harness = fs::read_to_string(root.join("benches/ckc_perf.rs")).unwrap();
    let vector = fs::read_to_string(root.join("benches/vector_perf.rs")).unwrap();
    let replay = fs::read_to_string(root.join("benches/runtime_replay.rs")).unwrap();
    let combined = format!("{harness}\n{vector}\n{replay}");
    for required in [
        "\\\"schemaVersion\\\": 7",
        "rotating-twelve-channel-v1",
        "interleaved-upper-median-three-channel-v2",
        "targetProfile",
        "runtimeReplayV011",
        "runtimeReplayV010",
        "vectorSuites",
        "domainFactSuites",
        "oracleIdentity",
        "oracleArtifacts",
        "artifactSizeComparisons",
        "compileTimeComparisons",
        "sample_three_channels",
        "KirTargetProfile",
        "build_kir_module_with_profile",
        "audit-performance-oracles.py",
        "ckc-v011",
        "--kind",
        "object",
    ] {
        assert!(
            combined.contains(required),
            "schema 7 producer must contain {required:?}"
        );
    }
}

#[test]
fn source_to_object_compile_time_should_measure_terminated_child_cpu_time() {
    let vector = fs::read_to_string(repo_root().join("benches/vector_perf.rs"))
        .expect("read vector performance harness");
    let compile_ck = &vector[vector.find("fn compile_ck(").expect("compile_ck")
        ..vector
            .find("#[cfg(unix)]\ntype CompileTimer")
            .expect("compile timer")];

    for required in [
        "RUSAGE_CHILDREN",
        "getrusage",
        "compile_timer_start()",
        "compile_timer_elapsed(timer)",
    ] {
        assert!(
            vector.contains(required),
            "source-to-object measurements must use terminated-child CPU time via {required:?}"
        );
    }
    assert!(
        !compile_ck.contains("Instant::now()"),
        "source-to-object measurements must exclude hosted-runner descheduling time"
    );
}

#[test]
fn native_performance_gate_should_enforce_equivalence_stability_and_thresholds() {
    let output = Command::new("python3")
        .arg("-B")
        .arg(repo_root().join("tests/performance/runtime_gate_test.py"))
        .output()
        .expect("run schema-7 checker regression suite");
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
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
