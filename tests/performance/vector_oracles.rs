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
}
