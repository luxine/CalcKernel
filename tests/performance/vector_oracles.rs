use std::{collections::BTreeSet, fs};

use sha2::{Digest, Sha256};

use super::support::oracle::repo_root;

const VECTOR_CASES: [&str; 8] = [
    "map_u32",
    "zip_u32",
    "strict_f64",
    "integer_cast",
    "modular_reduction",
    "slp_quad",
    "runtime_noalias",
    "specialized_length",
];
const DOMAIN_CASES: [&str; 2] = ["contract_noalias", "contract_fixed_length"];

#[test]
fn v012_oracle_manifest_should_pin_the_exact_corpus_sources_and_preconditions() {
    let root = repo_root();
    let manifest_path = root.join("benches/oracles/manifest.toml");
    let manifest = fs::read_to_string(&manifest_path).expect("read v0.12 oracle manifest");
    for required in [
        "schema_version = 1",
        "clang_version = \"22.1.8\"",
        "rust_version = \"1.90.0\"",
        "fast_math = false",
        "contraction = false",
        "builtin_library_calls = false",
        "sampling_protocol = \"rotating-three-channel-v1\"",
        "dispatch_protocol = \"cached-typed-entry-v1\"",
        "batch_iterations = 20000000",
        "sample_calls = 7",
        "sample_statistic = \"upper-median-of-seven\"",
        "short_kernel_conditioning = true",
        "short_kernel_conditioning_batches = 32",
        "differential_audit = true",
        "ub_audit = true",
    ] {
        assert!(
            manifest.contains(required),
            "oracle manifest must pin {required:?}"
        );
    }

    let mut declared = BTreeSet::new();
    for name in VECTOR_CASES.into_iter().chain(DOMAIN_CASES) {
        let fixture = format!("benches/oracles/fixtures/{name}.ck");
        let bytes = fs::read(root.join(&fixture)).expect("read CK oracle fixture");
        let digest = format!("{:x}", Sha256::digest(bytes));
        assert!(manifest.contains(&format!("name = \"{name}\"")));
        assert!(manifest.contains(&format!("ck_source = \"{fixture}\"")));
        assert!(manifest.contains(&format!("ck_sha256 = \"{digest}\"")));
        assert!(
            manifest.contains(&format!("valid_domain = \"{name}:")),
            "each kernel must freeze its valid input domain"
        );
        assert!(declared.insert(name));
    }
    assert_eq!(manifest.matches("[[kernel]]").count(), 10);

    for source in [
        "benches/oracles/c/vector_oracle.c",
        "benches/oracles/rust/vector_oracle.rs",
    ] {
        let bytes = fs::read(root.join(source)).expect("read oracle source");
        let digest = format!("{:x}", Sha256::digest(bytes));
        assert!(manifest.contains(&format!("source = \"{source}\"")));
        assert!(manifest.contains(&format!("sha256 = \"{digest}\"")));
    }
}

#[test]
fn modular_reduction_fixture_should_expose_its_valid_slice_domain_to_ck() {
    let source =
        fs::read_to_string(repo_root().join("benches/oracles/fixtures/modular_reduction.ck"))
            .expect("read modular reduction CK fixture");

    assert!(
        source.contains("export unsafe fn kernel"),
        "the reduction fixture must make its caller-owned valid domain explicit"
    );
    assert!(
        source.contains("requires n <= a.len"),
        "the fixed oracle domain must authorize CK to eliminate its redundant bounds guard"
    );
}

#[test]
fn oracle_sources_should_be_independent_and_architecture_explicit() {
    let root = repo_root();
    let c = fs::read_to_string(root.join("benches/oracles/c/vector_oracle.c"))
        .expect("read C SIMD oracle");
    let rust = fs::read_to_string(root.join("benches/oracles/rust/vector_oracle.rs"))
        .expect("read Rust SIMD oracle");
    for required in [
        "__attribute__((vector_size(16)))",
        "__builtin_memcpy",
        "ORACLE_CASE",
        "ck_oracle_kernel",
    ] {
        assert!(
            c.contains(required),
            "C SIMD oracle must contain {required:?}"
        );
    }
    for required in [
        "std::arch::x86_64",
        "std::arch::aarch64",
        "target_arch = \"x86_64\"",
        "target_arch = \"aarch64\"",
        "oracle_case",
        "ck_oracle_kernel",
    ] {
        assert!(
            rust.contains(required),
            "Rust SIMD oracle must contain {required:?}"
        );
    }
    for forbidden in ["calckernel", "emit_c_kir_module", "build_kir_module"] {
        assert!(!c.contains(forbidden));
        assert!(!rust.contains(forbidden));
    }
    for required in [
        "_mm_unpacklo_epi32(source, zero)",
        "0x4330_0000_0000_0000",
        "4_503_599_627_370_496.0",
    ] {
        assert!(
            rust.contains(required),
            "x86-64 u32-to-f64 SIMD must preserve the full unsigned domain via {required:?}"
        );
    }
    assert!(
        !rust.contains("_mm_cvtepi32_pd(source)"),
        "signed x86 conversion is not equivalent for u32 values with the high bit set"
    );
}

#[test]
fn oracle_audit_should_compile_both_languages_compare_every_mode_and_enable_ubsan() {
    let script = fs::read_to_string(repo_root().join("scripts/audit-performance-oracles.py"))
        .expect("read oracle audit");
    for required in [
        "-fsanitize=undefined",
        "-fno-sanitize-recover=all",
        "CKC_CLANG_ORACLE",
        "rustc 1.90.0",
        "for checked in (False, True)",
        "for kernel in manifest[\"kernel\"]",
        "compare_kernel",
        "oracle audit passed",
    ] {
        assert!(
            script.contains(required),
            "oracle audit must contain {required:?}"
        );
    }
}

#[test]
fn oracle_benchmark_should_cache_dispatch_before_the_timed_call_loop() {
    let harness = fs::read_to_string(repo_root().join("benches/vector_perf.rs"))
        .expect("read vector performance harness");
    let runner = harness
        .split("impl KernelRunner {")
        .nth(1)
        .and_then(|source| source.split("fn work_items(").next())
        .expect("KernelRunner implementation");

    assert!(
        runner.contains("KernelEntry::load(&library, symbol, name, checked)"),
        "the dynamic symbol and signature must be resolved while constructing the runner"
    );
    assert!(
        runner.contains("match self.entry"),
        "the timed batch must dispatch once to a cached typed entry"
    );
    for forbidden in [
        "self.library.symbol(self.symbol)",
        "unsafe fn invoke(&mut self)",
        "unsafe fn invoke_map(&mut self",
    ] {
        assert!(
            !runner.contains(forbidden),
            "the timed loop must not contain per-call lookup or string dispatch: {forbidden}"
        );
    }
    assert!(
        harness.contains("const SLP_CONDITIONING_BATCHES: usize = 32;")
            && runner.contains("if self.name == \"slp_quad\"")
            && runner.contains("for _ in 0..SLP_CONDITIONING_BATCHES")
            && runner.contains(
                "self.invoke_repeated(batch_iterations)?;\n            }\n        }\n        let timer = runtime_timer_start()?;"
            ),
        "the four-item SLP kernel must condition the same runner through a fixed 32-batch ramp before each timed sample"
    );
    assert!(
        harness.contains("sample_upper_median::<_, SAMPLE_REPETITIONS>")
            && !harness.contains("minimum.min(runners[channel].measure_once"),
        "each seven-call vector sample must reject minority scheduler outliers via its upper median"
    );
}

#[test]
fn linux_vector_runtime_gate_should_pin_one_allowed_cpu_before_conditioning() {
    let harness = fs::read_to_string(repo_root().join("benches/vector_perf.rs"))
        .expect("read vector performance harness");
    let measure_case = harness
        .split("fn measure_case(")
        .nth(1)
        .and_then(|source| source.split("type MapUnchecked").next())
        .expect("measure_case implementation");

    for required in [
        "#[cfg(target_os = \"linux\")]\nstruct LinuxCpuAffinityGuard",
        "libc::sched_getaffinity",
        "libc::CPU_ISSET",
        "libc::sched_setaffinity",
        "impl Drop for LinuxCpuAffinityGuard",
    ] {
        assert!(
            harness.contains(required),
            "the Linux performance gate must preserve and pin an allowed CPU via {required:?}"
        );
    }
    assert!(
        measure_case.contains("let _affinity = LinuxCpuAffinityGuard::pin_current()?;")
            && measure_case
                .find("LinuxCpuAffinityGuard::pin_current")
                .unwrap()
                < measure_case.find("KernelRunner::new").unwrap(),
        "the runtime gate must pin one allowed CPU before runner conditioning and timing"
    );
}

#[test]
fn linux_vector_runtime_gate_should_measure_current_thread_cpu_time() {
    let harness = fs::read_to_string(repo_root().join("benches/vector_perf.rs"))
        .expect("read vector performance harness");
    let measure_once = harness
        .split("fn measure_once(")
        .nth(1)
        .and_then(|source| source.split("fn run_batch(").next())
        .expect("measure_once implementation");

    for required in [
        "#[cfg(target_os = \"linux\")]\ntype RuntimeTimer = libc::timespec;",
        "libc::CLOCK_THREAD_CPUTIME_ID",
        "fn runtime_timer_start()",
        "fn runtime_timer_elapsed(timer: RuntimeTimer)",
        "let timer = runtime_timer_start()?;",
        "runtime_timer_elapsed(timer)?",
    ] {
        assert!(
            harness.contains(required),
            "the Linux runtime gate must use current-thread CPU time via {required:?}"
        );
    }
    assert!(
        !measure_once.contains("Instant::now"),
        "the authoritative Linux runtime sample must not directly use parent wall-clock time"
    );
}
