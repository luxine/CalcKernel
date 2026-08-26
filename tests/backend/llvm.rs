use std::{
    fs,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

use calckernel::{
    EmitLlvmOptions, MirPassBoundsMode, MirPassOverflowMode, MirPassTargetBackend, emit_llvm_module,
};

use super::support::fixtures;
use super::support::oracle::{typescript_cli, typescript_root};
use super::support::{
    command::{clang_available, python3_available},
    compiler::{optimized_module, shared_library_path},
};

fn emit_llvm(source_text: &str) -> String {
    emit_llvm_with_opt_level(source_text, 1)
}

fn emit_llvm_with_opt_level(source_text: &str, opt_level: u8) -> String {
    let optimized = optimized_module(
        source_text,
        opt_level,
        MirPassOverflowMode::Unchecked,
        MirPassBoundsMode::Unchecked,
        MirPassTargetBackend::Llvm,
    );
    emit_llvm_module(
        &optimized,
        &EmitLlvmOptions {
            source_file_name: Some("test.ck".to_string()),
            target_triple: None,
        },
    )
}

#[test]
fn llvm_backend_should_compile_and_run_scalar_control_and_memory_program() {
    let ir = emit_llvm(
        r#"
      struct Item {
        price: i64;
        qty: i64;
      }

      export fn add_i64(a: i64, b: i64) -> i64 {
        return a + b;
      }

      export fn sum_to_n(n: i64) -> i64 {
        let i: i64 = 0;
        let sum: i64 = 0;
        while i < n {
          sum = sum + i;
          i = i + 1;
        }
        return sum;
      }

      export fn calc(items: ptr<Item>, out: ptr<i64>) -> i32 {
        out[0] = items[0].price * items[0].qty;
        return 0;
      }

      export fn as_f64(a: i32, b: u32) -> f64 {
        return i32_to_f64(a) + u32_to_f64(b);
      }
    "#,
    );

    let harness = r#"
#include <stdint.h>

typedef struct Item {
  int64_t price;
  int64_t qty;
} Item;

int64_t add_i64(int64_t a, int64_t b);
int64_t sum_to_n(int64_t n);
int32_t calc(Item *items, int64_t *out);
double as_f64(int32_t a, uint32_t b);

int main(void) {
  if (add_i64(2, 3) != 5) return 1;
  if (sum_to_n(5) != 10) return 2;
  Item items[1] = { { 7, 6 } };
  int64_t out[1] = {0};
  if (calc(items, out) != 0) return 3;
  if (out[0] != 42) return 4;
  if (as_f64(-2, 5) != 3.0) return 5;
  return 0;
}
"#;

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("rust_calckernel_llvm_backend_{unique}"));
    fs::create_dir_all(&dir).expect("create temp dir");
    let ir_path = dir.join("module.ll");
    let harness_path = dir.join("harness.c");
    let bin_path = dir.join("harness");
    fs::write(&ir_path, ir).expect("write ir");
    fs::write(&harness_path, harness).expect("write harness");

    let compile = Command::new("clang")
        .arg(&ir_path)
        .arg(&harness_path)
        .arg("-Wno-override-module")
        .arg("-o")
        .arg(&bin_path)
        .output()
        .expect("run clang");
    assert!(
        compile.status.success(),
        "clang failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&compile.stdout),
        String::from_utf8_lossy(&compile.stderr)
    );

    let run = Command::new(&bin_path).output().expect("run harness");
    assert!(
        run.status.success(),
        "harness failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
}

#[test]
fn llvm_backend_should_match_typescript_oracle_for_official_examples() {
    let Some(ts_cli) = typescript_cli() else {
        return;
    };
    let target = "ck-test-target";
    let examples = fixtures::ORACLE_EXAMPLES
        .iter()
        .chain(fixtures::BENCHMARK_FIXTURES)
        .map(|fixture| fixture.oracle)
        .collect::<Vec<_>>();

    for example in examples {
        let source = typescript_root().join(example);

        let ts_output = Command::new("node")
            .arg(&ts_cli)
            .arg("emit-llvm")
            .arg("--target")
            .arg(target)
            .arg(&source)
            .output()
            .expect("run TypeScript emit-llvm");
        assert!(
            ts_output.status.success(),
            "{example} TS stderr:\n{}",
            String::from_utf8_lossy(&ts_output.stderr)
        );

        let rust_output = Command::new(env!("CARGO_BIN_EXE_ckc"))
            .arg("emit-llvm")
            .arg("--target")
            .arg(target)
            .arg(&source)
            .output()
            .expect("run Rust emit-llvm");
        assert!(
            rust_output.status.success(),
            "{example} Rust stderr:\n{}",
            String::from_utf8_lossy(&rust_output.stderr)
        );

        assert_eq!(
            String::from_utf8(rust_output.stdout).expect("Rust LLVM should be UTF-8"),
            String::from_utf8(ts_output.stdout).expect("TS LLVM should be UTF-8"),
            "{example}"
        );
    }
}

#[test]
fn llvm_backend_should_match_typescript_oracle_for_f64_edge_fixture() {
    let Some(ts_cli) = typescript_cli() else {
        return;
    };
    let source = typescript_root().join("tests/fixtures/f64_edges.ck");
    let target = "ck-test-target";

    let ts_output = Command::new("node")
        .arg(&ts_cli)
        .arg("emit-llvm")
        .arg("--target")
        .arg(target)
        .arg(&source)
        .output()
        .expect("run TypeScript emit-llvm");
    assert!(
        ts_output.status.success(),
        "TS stderr:\n{}",
        String::from_utf8_lossy(&ts_output.stderr)
    );

    let rust_output = Command::new(env!("CARGO_BIN_EXE_ckc"))
        .arg("emit-llvm")
        .arg("--target")
        .arg(target)
        .arg(&source)
        .output()
        .expect("run Rust emit-llvm");
    assert!(
        rust_output.status.success(),
        "Rust stderr:\n{}",
        String::from_utf8_lossy(&rust_output.stderr)
    );

    let ts_ir = String::from_utf8(ts_output.stdout).expect("TS LLVM should be UTF-8");
    let rust_ir = String::from_utf8(rust_output.stdout).expect("Rust LLVM should be UTF-8");
    assert!(
        ts_ir.contains("fcmp une double"),
        "TS oracle should use unordered f64 not-equal"
    );
    assert_eq!(rust_ir, ts_ir);
}

#[test]
fn llvm_backend_should_match_typescript_oracle_for_perf_f64_kernels_at_o3() {
    let Some(ts_cli) = typescript_cli() else {
        return;
    };
    let source = typescript_root().join(fixtures::BENCH_F64_KERNELS.oracle);
    let target = "ck-test-target";

    let ts_output = Command::new("node")
        .arg(&ts_cli)
        .arg("emit-llvm")
        .arg("--target")
        .arg(target)
        .arg("-O3")
        .arg(&source)
        .output()
        .expect("run TypeScript emit-llvm");
    assert!(
        ts_output.status.success(),
        "TS stderr:\n{}",
        String::from_utf8_lossy(&ts_output.stderr)
    );

    let rust_output = Command::new(env!("CARGO_BIN_EXE_ckc"))
        .arg("emit-llvm")
        .arg("--target")
        .arg(target)
        .arg("-O3")
        .arg(&source)
        .output()
        .expect("run Rust emit-llvm");
    assert!(
        rust_output.status.success(),
        "Rust stderr:\n{}",
        String::from_utf8_lossy(&rust_output.stderr)
    );

    assert_eq!(
        String::from_utf8(rust_output.stdout).expect("Rust LLVM should be UTF-8"),
        String::from_utf8(ts_output.stdout).expect("TS LLVM should be UTF-8")
    );
}

#[test]
fn llvm_cli_should_match_typescript_oracle_for_default_target_detection() {
    let Some(ts_cli) = typescript_cli() else {
        return;
    };
    let source = typescript_root().join(fixtures::CORE_SCALAR.oracle);

    let ts_output = Command::new("node")
        .arg(ts_cli)
        .arg("emit-llvm")
        .arg(&source)
        .output()
        .expect("run TypeScript emit-llvm");
    assert!(
        ts_output.status.success(),
        "TS stderr:\n{}",
        String::from_utf8_lossy(&ts_output.stderr)
    );

    let rust_output = Command::new(env!("CARGO_BIN_EXE_ckc"))
        .arg("emit-llvm")
        .arg(&source)
        .output()
        .expect("run Rust emit-llvm");
    assert!(
        rust_output.status.success(),
        "Rust stderr:\n{}",
        String::from_utf8_lossy(&rust_output.stderr)
    );

    assert_eq!(
        String::from_utf8(rust_output.stdout).expect("Rust LLVM should be UTF-8"),
        String::from_utf8(ts_output.stdout).expect("TS LLVM should be UTF-8")
    );
}

#[test]
fn build_llvm_should_match_typescript_oracle_without_default_target_detection() {
    let Some(ts_cli) = typescript_cli() else {
        return;
    };
    if !clang_available() {
        return;
    }

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("rust_calckernel_build_llvm_oracle_{unique}"));
    let ts_dir = dir.join("ts");
    let rust_dir = dir.join("rust");
    fs::create_dir_all(&ts_dir).expect("create TS output dir");
    fs::create_dir_all(&rust_dir).expect("create Rust output dir");
    let source = typescript_root().join(fixtures::CORE_SCALAR.oracle);
    let ts_object = ts_dir.join("scalar.o");
    let rust_object = rust_dir.join("scalar.o");

    let ts_output = Command::new("node")
        .arg(&ts_cli)
        .arg("build-llvm")
        .arg("--kind")
        .arg("object")
        .arg("--out")
        .arg(&ts_object)
        .arg(&source)
        .output()
        .expect("run TypeScript build-llvm");
    assert!(
        ts_output.status.success(),
        "TS stderr:\n{}",
        String::from_utf8_lossy(&ts_output.stderr)
    );

    let rust_output = Command::new(env!("CARGO_BIN_EXE_ckc"))
        .arg("build-llvm")
        .arg("--kind")
        .arg("object")
        .arg("--out")
        .arg(&rust_object)
        .arg(&source)
        .output()
        .expect("run Rust build-llvm");
    assert!(
        rust_output.status.success(),
        "Rust stderr:\n{}",
        String::from_utf8_lossy(&rust_output.stderr)
    );

    assert_eq!(
        fs::read_to_string(rust_object.with_extension("ll")).expect("read Rust .ll"),
        fs::read_to_string(ts_object.with_extension("ll")).expect("read TS .ll")
    );
}

#[test]
fn build_llvm_object_should_match_typescript_oracle_for_pricing_runtime_behavior() {
    let Some(ts_cli) = typescript_cli() else {
        return;
    };
    if !clang_available() {
        return;
    }

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("rust_calckernel_build_llvm_runtime_{unique}"));
    let ts_dir = dir.join("ts");
    let rust_dir = dir.join("rust");
    fs::create_dir_all(&ts_dir).expect("create TS output dir");
    fs::create_dir_all(&rust_dir).expect("create Rust output dir");
    let harness_path = dir.join("pricing_harness.c");
    fs::write(&harness_path, pricing_llvm_object_harness()).expect("write pricing harness");

    let source = typescript_root().join(fixtures::APPLICATION_PRICING.oracle);
    let ts_object = ts_dir.join("pricing.o");
    let rust_object = rust_dir.join("pricing.o");

    let ts_output = Command::new("node")
        .arg(&ts_cli)
        .arg("build-llvm")
        .arg("--kind")
        .arg("object")
        .arg("--out")
        .arg(&ts_object)
        .arg(&source)
        .output()
        .expect("run TypeScript build-llvm");
    assert!(
        ts_output.status.success(),
        "TS stderr:\n{}",
        String::from_utf8_lossy(&ts_output.stderr)
    );

    let rust_output = Command::new(env!("CARGO_BIN_EXE_ckc"))
        .arg("build-llvm")
        .arg("--kind")
        .arg("object")
        .arg("--out")
        .arg(&rust_object)
        .arg(&source)
        .output()
        .expect("run Rust build-llvm");
    assert!(
        rust_output.status.success(),
        "Rust stderr:\n{}",
        String::from_utf8_lossy(&rust_output.stderr)
    );

    let ts_run =
        compile_and_run_llvm_object(&ts_object, &harness_path, &ts_dir.join("pricing_run"));
    let rust_run =
        compile_and_run_llvm_object(&rust_object, &harness_path, &rust_dir.join("pricing_run"));

    assert_eq!(rust_run, ts_run);
}

#[test]
fn build_llvm_object_should_match_typescript_oracle_for_official_e2e_runtime_behavior() {
    let Some(ts_cli) = typescript_cli() else {
        return;
    };
    if !clang_available() {
        return;
    }

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "rust_calckernel_build_llvm_official_runtime_{unique}"
    ));
    let ts_dir = dir.join("ts");
    let rust_dir = dir.join("rust");
    fs::create_dir_all(&ts_dir).expect("create TS output dir");
    fs::create_dir_all(&rust_dir).expect("create Rust output dir");
    let cases = [
        ("llvm-scalar", fixtures::LLVM_SCALAR.oracle),
        ("llvm-calls", fixtures::LLVM_CALLS.oracle),
        ("llvm-control-flow", fixtures::LLVM_CONTROL_FLOW.oracle),
        ("llvm-memory", fixtures::LLVM_MEMORY.oracle),
        ("llvm-short-circuit", fixtures::LLVM_SHORT_CIRCUIT.oracle),
        ("llvm-bool", fixtures::LLVM_BOOL.oracle),
        ("llvm-dijkstra", fixtures::APPLICATION_DIJKSTRA.oracle),
        ("llvm-f64-array", fixtures::WASM_F64_ARRAY.oracle),
        ("llvm-f64-axpy", fixtures::WASM_F64_AXPY.oracle),
        ("llvm-f64-sum", fixtures::WASM_F64_SUM.oracle),
        ("llvm-pricing-soa", fixtures::WASM_PRICING_SOA.oracle),
        ("llvm-f64-edges", "tests/fixtures/f64_edges.ck"),
    ];

    for (case_name, example) in cases {
        let source = typescript_root().join(example);
        let harness_path = dir.join(format!("{case_name}.c"));
        fs::write(&harness_path, official_llvm_object_harness(case_name))
            .expect("write official LLVM harness");
        let ts_object = ts_dir.join(format!("{case_name}.o"));
        let rust_object = rust_dir.join(format!("{case_name}.o"));

        let ts_output = Command::new("node")
            .arg(&ts_cli)
            .arg("build-llvm")
            .arg("--kind")
            .arg("object")
            .arg("--out")
            .arg(&ts_object)
            .arg(&source)
            .output()
            .expect("run TypeScript build-llvm");
        assert!(
            ts_output.status.success(),
            "{case_name} TS stderr:\n{}",
            String::from_utf8_lossy(&ts_output.stderr)
        );

        let rust_output = Command::new(env!("CARGO_BIN_EXE_ckc"))
            .arg("build-llvm")
            .arg("--kind")
            .arg("object")
            .arg("--out")
            .arg(&rust_object)
            .arg(&source)
            .output()
            .expect("run Rust build-llvm");
        assert!(
            rust_output.status.success(),
            "{case_name} Rust stderr:\n{}",
            String::from_utf8_lossy(&rust_output.stderr)
        );

        let ts_run =
            compile_and_run_llvm_object(&ts_object, &harness_path, &ts_dir.join(case_name));
        let rust_run =
            compile_and_run_llvm_object(&rust_object, &harness_path, &rust_dir.join(case_name));

        assert_eq!(rust_run, ts_run, "{case_name}");
    }
}

#[test]
fn build_llvm_object_should_match_typescript_oracle_for_perf_f64_kernels_runtime_behavior() {
    let Some(ts_cli) = typescript_cli() else {
        return;
    };
    if !clang_available() {
        return;
    }

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "rust_calckernel_build_llvm_perf_f64_runtime_{unique}"
    ));
    let ts_dir = dir.join("ts");
    let rust_dir = dir.join("rust");
    fs::create_dir_all(&ts_dir).expect("create TS output dir");
    fs::create_dir_all(&rust_dir).expect("create Rust output dir");
    let case_name = "llvm-perf-f64-kernels";
    let source = typescript_root().join(fixtures::BENCH_F64_KERNELS.oracle);
    let harness_path = dir.join("perf_f64_kernels.c");
    fs::write(&harness_path, official_llvm_object_harness(case_name))
        .expect("write perf f64 LLVM harness");
    let ts_object = ts_dir.join("perf_f64_kernels.o");
    let rust_object = rust_dir.join("perf_f64_kernels.o");

    let ts_output = Command::new("node")
        .arg(&ts_cli)
        .arg("build-llvm")
        .arg("--kind")
        .arg("object")
        .arg("--out")
        .arg(&ts_object)
        .arg("-O3")
        .arg(&source)
        .output()
        .expect("run TypeScript build-llvm");
    assert!(
        ts_output.status.success(),
        "TS stderr:\n{}",
        String::from_utf8_lossy(&ts_output.stderr)
    );

    let rust_output = Command::new(env!("CARGO_BIN_EXE_ckc"))
        .arg("build-llvm")
        .arg("--kind")
        .arg("object")
        .arg("--out")
        .arg(&rust_object)
        .arg("-O3")
        .arg(&source)
        .output()
        .expect("run Rust build-llvm");
    assert!(
        rust_output.status.success(),
        "Rust stderr:\n{}",
        String::from_utf8_lossy(&rust_output.stderr)
    );
    assert_eq!(
        fs::read_to_string(rust_object.with_extension("ll")).expect("read Rust .ll"),
        fs::read_to_string(ts_object.with_extension("ll")).expect("read TS .ll")
    );

    let ts_run = compile_and_run_llvm_object(&ts_object, &harness_path, &ts_dir.join(case_name));
    let rust_run =
        compile_and_run_llvm_object(&rust_object, &harness_path, &rust_dir.join(case_name));

    assert_eq!(rust_run, ts_run);
}

#[test]
fn build_llvm_dynamic_should_match_typescript_oracle_for_pricing_runtime_behavior() {
    let Some(ts_cli) = typescript_cli() else {
        return;
    };
    if !clang_available() || !python3_available() {
        return;
    }

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "rust_calckernel_build_llvm_dynamic_runtime_{unique}"
    ));
    let ts_dir = dir.join("ts");
    let rust_dir = dir.join("rust");
    fs::create_dir_all(&ts_dir).expect("create TS output dir");
    fs::create_dir_all(&rust_dir).expect("create Rust output dir");
    let runner = dir.join("run_pricing_llvm_dynamic.py");
    fs::write(&runner, pricing_llvm_dynamic_runner()).expect("write dynamic runner");

    let source = typescript_root().join(fixtures::APPLICATION_PRICING.oracle);
    let ts_base = ts_dir.join("pricing_llvm_dynamic");
    let rust_base = rust_dir.join("pricing_llvm_dynamic");

    let ts_output = Command::new("node")
        .arg(&ts_cli)
        .arg("build-llvm")
        .arg("--kind")
        .arg("dynamic")
        .arg("--out")
        .arg(&ts_base)
        .arg(&source)
        .output()
        .expect("run TypeScript build-llvm dynamic");
    assert!(
        ts_output.status.success(),
        "TS stderr:\n{}",
        String::from_utf8_lossy(&ts_output.stderr)
    );

    let rust_output = Command::new(env!("CARGO_BIN_EXE_ckc"))
        .arg("build-llvm")
        .arg("--kind")
        .arg("dynamic")
        .arg("--out")
        .arg(&rust_base)
        .arg(&source)
        .output()
        .expect("run Rust build-llvm dynamic");
    assert!(
        rust_output.status.success(),
        "Rust stderr:\n{}",
        String::from_utf8_lossy(&rust_output.stderr)
    );

    let ts_run = run_python_pricing_llvm_dynamic(&runner, &shared_library_path(&ts_base));
    let rust_run = run_python_pricing_llvm_dynamic(&runner, &shared_library_path(&rust_base));

    assert_eq!(rust_run, ts_run);
}

#[test]
fn build_llvm_dynamic_should_match_typescript_oracle_for_official_e2e_runtime_behavior() {
    let Some(ts_cli) = typescript_cli() else {
        return;
    };
    if !clang_available() || !python3_available() {
        return;
    }

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "rust_calckernel_build_llvm_official_dynamic_runtime_{unique}"
    ));
    let ts_dir = dir.join("ts");
    let rust_dir = dir.join("rust");
    fs::create_dir_all(&ts_dir).expect("create TS output dir");
    fs::create_dir_all(&rust_dir).expect("create Rust output dir");
    let runner = dir.join("run_official_llvm_dynamic.py");
    fs::write(&runner, official_llvm_dynamic_runner()).expect("write official dynamic runner");
    let cases = [
        ("llvm-scalar", fixtures::LLVM_SCALAR.oracle),
        ("llvm-calls", fixtures::LLVM_CALLS.oracle),
        ("llvm-control-flow", fixtures::LLVM_CONTROL_FLOW.oracle),
        ("llvm-memory", fixtures::LLVM_MEMORY.oracle),
        ("llvm-short-circuit", fixtures::LLVM_SHORT_CIRCUIT.oracle),
        ("llvm-bool", fixtures::LLVM_BOOL.oracle),
        ("llvm-dijkstra", fixtures::APPLICATION_DIJKSTRA.oracle),
        ("llvm-f64-array", fixtures::WASM_F64_ARRAY.oracle),
        ("llvm-f64-axpy", fixtures::WASM_F64_AXPY.oracle),
        ("llvm-f64-sum", fixtures::WASM_F64_SUM.oracle),
        ("llvm-pricing-soa", fixtures::WASM_PRICING_SOA.oracle),
        ("llvm-f64-edges", "tests/fixtures/f64_edges.ck"),
    ];

    for (case_name, example) in cases {
        let source = typescript_root().join(example);
        let ts_base = ts_dir.join(case_name);
        let rust_base = rust_dir.join(case_name);

        let ts_output = Command::new("node")
            .arg(&ts_cli)
            .arg("build-llvm")
            .arg("--kind")
            .arg("dynamic")
            .arg("--out")
            .arg(&ts_base)
            .arg(&source)
            .output()
            .expect("run TypeScript build-llvm dynamic");
        assert!(
            ts_output.status.success(),
            "{case_name} TS stderr:\n{}",
            String::from_utf8_lossy(&ts_output.stderr)
        );

        let rust_output = Command::new(env!("CARGO_BIN_EXE_ckc"))
            .arg("build-llvm")
            .arg("--kind")
            .arg("dynamic")
            .arg("--out")
            .arg(&rust_base)
            .arg(&source)
            .output()
            .expect("run Rust build-llvm dynamic");
        assert!(
            rust_output.status.success(),
            "{case_name} Rust stderr:\n{}",
            String::from_utf8_lossy(&rust_output.stderr)
        );

        let ts_run =
            run_python_official_llvm_dynamic(&runner, case_name, &shared_library_path(&ts_base));
        let rust_run =
            run_python_official_llvm_dynamic(&runner, case_name, &shared_library_path(&rust_base));

        assert_eq!(
            ts_run.status_code,
            Some(0),
            "{case_name} TS runtime stderr: {}",
            ts_run.stderr
        );
        assert_eq!(
            rust_run.status_code,
            Some(0),
            "{case_name} Rust runtime stderr: {}",
            rust_run.stderr
        );
        assert_eq!(rust_run, ts_run, "{case_name}");
    }
}

#[test]
fn build_llvm_dynamic_should_match_typescript_oracle_for_perf_f64_kernels_runtime_behavior() {
    let Some(ts_cli) = typescript_cli() else {
        return;
    };
    if !clang_available() || !python3_available() {
        return;
    }

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "rust_calckernel_build_llvm_perf_f64_dynamic_runtime_{unique}"
    ));
    let ts_dir = dir.join("ts");
    let rust_dir = dir.join("rust");
    fs::create_dir_all(&ts_dir).expect("create TS output dir");
    fs::create_dir_all(&rust_dir).expect("create Rust output dir");
    let runner = dir.join("run_perf_f64_llvm_dynamic.py");
    fs::write(&runner, official_llvm_dynamic_runner()).expect("write perf dynamic runner");
    let case_name = "llvm-perf-f64-kernels";
    let source = typescript_root().join(fixtures::BENCH_F64_KERNELS.oracle);
    let ts_base = ts_dir.join(case_name);
    let rust_base = rust_dir.join(case_name);

    let ts_output = Command::new("node")
        .arg(&ts_cli)
        .arg("build-llvm")
        .arg("--kind")
        .arg("dynamic")
        .arg("--out")
        .arg(&ts_base)
        .arg("-O3")
        .arg(&source)
        .output()
        .expect("run TypeScript build-llvm dynamic");
    assert!(
        ts_output.status.success(),
        "TS stderr:\n{}",
        String::from_utf8_lossy(&ts_output.stderr)
    );

    let rust_output = Command::new(env!("CARGO_BIN_EXE_ckc"))
        .arg("build-llvm")
        .arg("--kind")
        .arg("dynamic")
        .arg("--out")
        .arg(&rust_base)
        .arg("-O3")
        .arg(&source)
        .output()
        .expect("run Rust build-llvm dynamic");
    assert!(
        rust_output.status.success(),
        "Rust stderr:\n{}",
        String::from_utf8_lossy(&rust_output.stderr)
    );

    let ts_run =
        run_python_official_llvm_dynamic(&runner, case_name, &shared_library_path(&ts_base));
    let rust_run =
        run_python_official_llvm_dynamic(&runner, case_name, &shared_library_path(&rust_base));

    assert_eq!(ts_run.status_code, Some(0), "TS stderr: {}", ts_run.stderr);
    assert_eq!(
        rust_run.status_code,
        Some(0),
        "Rust stderr: {}",
        rust_run.stderr
    );
    assert_eq!(rust_run, ts_run);
}

#[test]
fn llvm_backend_should_run_nested_break_continue_at_all_opt_levels() {
    if !clang_available() {
        return;
    }
    let source = r#"
      export fn early_exit(n: u32) -> u32 {
        let i: u32 = 0;
        while i < n {
          if i == 3 {
            break;
          }
          i = i + 1;
        }
        return i;
      }

      export fn skip_three(n: u32) -> u32 {
        let i: u32 = 0;
        let sum: u32 = 0;
        while i < n {
          i = i + 1;
          if i == 3 {
            continue;
          }
          sum = sum + i;
        }
        return sum;
      }

      export fn nested(n: u32) -> u32 {
        let outer: u32 = 0;
        let hits: u32 = 0;
        while outer < n {
          let inner: u32 = 0;
          while inner < n {
            inner = inner + 1;
            if inner == 2 {
              continue;
            }
            hits = hits + 1;
            if inner == 3 {
              break;
            }
          }
          outer = outer + 1;
        }
        return hits;
      }

      export fn return_from_loop(n: u32) -> u32 {
        let i: u32 = 0;
        while i < n {
          if i == 2 {
            return 99;
          }
          i = i + 1;
          if i == 1 {
            continue;
          }
        }
        return i;
      }
    "#;
    let harness = r#"
#include <stdint.h>

uint32_t early_exit(uint32_t n);
uint32_t skip_three(uint32_t n);
uint32_t nested(uint32_t n);
uint32_t return_from_loop(uint32_t n);

int main(void) {
  if (early_exit(10) != 3) return 1;
  if (skip_three(5) != 12) return 2;
  if (nested(4) != 8) return 3;
  if (return_from_loop(5) != 99) return 4;
  return 0;
}
"#;
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("rust_calckernel_break_continue_llvm_{unique}"));
    fs::create_dir_all(&dir).expect("create temp dir");

    for opt_level in 0..=3 {
        let ir_path = dir.join(format!("control_o{opt_level}.ll"));
        let harness_path = dir.join("harness.c");
        let bin_path = dir.join(format!("harness_o{opt_level}"));
        fs::write(&ir_path, emit_llvm_with_opt_level(source, opt_level)).expect("write LLVM IR");
        fs::write(&harness_path, harness).expect("write harness");
        let compile = Command::new("clang")
            .arg(&ir_path)
            .arg(&harness_path)
            .arg("-Wno-override-module")
            .arg("-o")
            .arg(&bin_path)
            .output()
            .expect("run clang");
        assert!(
            compile.status.success(),
            "O{opt_level} clang stderr:\n{}",
            String::from_utf8_lossy(&compile.stderr)
        );
        let run = Command::new(&bin_path).output().expect("run LLVM harness");
        assert!(
            run.status.success(),
            "O{opt_level} runtime stderr:\n{}",
            String::from_utf8_lossy(&run.stderr)
        );
    }
}

#[test]
fn llvm_backend_should_emit_call_and_ret_void() {
    if !clang_available() {
        return;
    }
    let ir = emit_llvm(
        r#"
      fn set_first(out: ptr<i32>) -> void { out[0] = 41; }
      export fn mutate(out: ptr<i32>, stop: bool) -> void {
        if stop { return; }
        set_first(out);
        out[0] = out[0] + 1;
      }
    "#,
    );
    assert!(ir.contains("define internal void @set_first"), "{ir}");
    assert!(ir.contains("define void @mutate"), "{ir}");
    assert!(ir.contains("call void @set_first"), "{ir}");
    assert!(ir.contains("ret void"), "{ir}");

    let harness = r#"
#include <stdbool.h>
#include <stdint.h>
void mutate(int32_t *out, bool stop);
int main(void) {
  int32_t value = 0;
  mutate(&value, false);
  if (value != 42) return 1;
  mutate(&value, true);
  return value == 42 ? 0 : 2;
}
"#;
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("rust_calckernel_void_llvm_{unique}"));
    fs::create_dir_all(&dir).expect("create temp dir");
    let ir_path = dir.join("void.ll");
    let harness_path = dir.join("harness.c");
    let binary = dir.join("void");
    fs::write(&ir_path, ir).expect("write ir");
    fs::write(&harness_path, harness).expect("write harness");
    let compile = Command::new("clang")
        .arg(&ir_path)
        .arg(&harness_path)
        .arg("-Wno-override-module")
        .arg("-o")
        .arg(&binary)
        .output()
        .expect("run clang");
    assert!(
        compile.status.success(),
        "clang stderr:\n{}",
        String::from_utf8_lossy(&compile.stderr)
    );
    assert!(
        Command::new(binary)
            .status()
            .expect("run harness")
            .success()
    );
}

#[test]
fn llvm_backend_should_flatten_slice_params_and_reconstruct_aggregate_values() {
    let ir = emit_llvm(
        r#"
      export fn length(items: slice<i32>) -> u32 {
        return items.len;
      }
    "#,
    );

    assert!(
        ir.contains("define i32 @length(ptr %items.data, i32 %items.len)"),
        "{ir}"
    );
    assert!(ir.contains("%items.addr = alloca { ptr, i32 }"), "{ir}");
    assert!(
        ir.contains("insertvalue { ptr, i32 } undef, ptr %items.data, 0"),
        "{ir}"
    );
    assert!(ir.contains("insertvalue { ptr, i32 }"), "{ir}");
    assert!(ir.contains("store { ptr, i32 }"), "{ir}");
}

#[test]
fn llvm_backend_should_emit_slice_moves_fields_and_internal_aggregate_returns() {
    let ir = emit_llvm(
        r#"
      struct Holder {
        items: slice<i32>;
      }

      fn identity(items: slice<i32>) -> slice<i32> {
        let copy: slice<i32> = items;
        return copy;
      }

      export fn round_trip(holder: ptr<Holder>, items: slice<i32>) -> u32 {
        holder[0].items = items;
        let stored: slice<i32> = holder[0].items;
        let returned: slice<i32> = identity(stored);
        return returned.len;
      }
    "#,
    );

    assert!(
        ir.contains("%struct.Holder = type { { ptr, i32 } }"),
        "{ir}"
    );
    assert!(
        ir.contains("define internal { ptr, i32 } @identity(ptr %items.data, i32 %items.len)"),
        "{ir}"
    );
    assert!(ir.contains("load { ptr, i32 }, ptr"), "{ir}");
    assert!(ir.contains("store { ptr, i32 }"), "{ir}");
    assert!(ir.contains("call { ptr, i32 } @identity(ptr"), "{ir}");
    assert!(ir.contains("ret { ptr, i32 }"), "{ir}");
}

#[test]
fn llvm_backend_should_gep_slice_indices_and_subslices() {
    let ir = emit_llvm(
        r#"
      struct Pair {
        left: i32;
        right: i32;
      }

      export fn selected(items: slice<Pair>, start: u32, end: u32) -> i32 {
        let middle: slice<Pair> = items[start..end];
        return middle[0].right;
      }
    "#,
    );

    assert!(ir.contains("getelementptr %struct.Pair, ptr"), "{ir}");
    assert!(ir.contains("icmp eq i32"), "{ir}");
    assert!(ir.contains("select i1"), "{ir}");
    assert!(ir.contains("extractvalue { ptr, i32 }"), "{ir}");
    assert!(!ir.contains("getelementptr { ptr, i32 }"), "{ir}");
}

#[test]
fn llvm_backend_should_run_slice_programs_at_all_opt_levels() {
    if !clang_available() {
        return;
    }
    let source = r#"
      struct Pair {
        left: i32;
        right: i32;
      }

      struct Holder {
        items: slice<Pair>;
      }

      fn choose(items: slice<Pair>, second: bool) -> slice<Pair> {
        if second {
          return items[1..2];
        }
        return items[0..1];
      }

      export fn round_trip(v0: i32, holder: ptr<Holder>, items: slice<Pair>, second: bool) -> i32 {
        holder[0].items = items;
        let stored: slice<Pair> = holder[0].items;
        let selected: slice<Pair> = choose(stored, second);
        return selected[0].left + selected[0].right + v0;
      }

      export fn empty_data(items: slice<Pair>) -> ptr<Pair> {
        let empty: slice<Pair> = items[0..0];
        return empty.data;
      }
    "#;
    let harness = r#"
#include <stdbool.h>
#include <stdint.h>

typedef struct Pair { int32_t left; int32_t right; } Pair;
typedef struct PairSlice { Pair* data; uint32_t len; } PairSlice;
typedef struct Holder { PairSlice items; } Holder;

int32_t round_trip(int32_t v0, Holder* holder, Pair* items_data, uint32_t items_len, bool second);
Pair* empty_data(Pair* items_data, uint32_t items_len);

int main(void) {
  Pair values[2] = {{2, 3}, {7, 11}};
  Holder holder = {0};
  if (round_trip(0, &holder, values, 2, false) != 5) return 1;
  if (round_trip(0, &holder, values, 2, true) != 18) return 2;
  if (holder.items.data != values || holder.items.len != 2) return 3;
  if (empty_data(values, 0) != values) return 4;
  return 0;
}
"#;
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("rust_calckernel_slice_llvm_{unique}"));
    fs::create_dir_all(&dir).expect("create temp dir");
    let harness_path = dir.join("harness.c");
    fs::write(&harness_path, harness).expect("write harness");

    for opt_level in 0..=3 {
        let ir_path = dir.join(format!("slice_o{opt_level}.ll"));
        let binary = dir.join(format!("slice_o{opt_level}"));
        fs::write(&ir_path, emit_llvm_with_opt_level(source, opt_level)).expect("write LLVM IR");
        let compile = Command::new("clang")
            .arg(&ir_path)
            .arg(&harness_path)
            .arg("-std=c11")
            .arg("-Wall")
            .arg("-Wextra")
            .arg("-Werror")
            .arg("-Wno-override-module")
            .arg("-o")
            .arg(&binary)
            .output()
            .expect("run clang");
        assert!(
            compile.status.success(),
            "O{opt_level} clang stderr:\n{}",
            String::from_utf8_lossy(&compile.stderr)
        );
        let run = Command::new(&binary).output().expect("run LLVM harness");
        assert!(
            run.status.success(),
            "O{opt_level} runtime exit={:?} stderr:\n{}",
            run.status.code(),
            String::from_utf8_lossy(&run.stderr)
        );
    }
}

#[derive(Debug, PartialEq, Eq)]
struct RuntimeOutput {
    status_code: Option<i32>,
    stdout: String,
    stderr: String,
}

fn run_python_pricing_llvm_dynamic(
    runner: &std::path::Path,
    library_path: &std::path::Path,
) -> RuntimeOutput {
    let output = Command::new("python3")
        .arg(runner)
        .arg(library_path)
        .output()
        .expect("run pricing LLVM dynamic runner");
    RuntimeOutput {
        status_code: output.status.code(),
        stdout: String::from_utf8(output.stdout).expect("runtime stdout should be UTF-8"),
        stderr: String::from_utf8(output.stderr).expect("runtime stderr should be UTF-8"),
    }
}

fn run_python_official_llvm_dynamic(
    runner: &std::path::Path,
    case_name: &str,
    library_path: &std::path::Path,
) -> RuntimeOutput {
    let output = Command::new("python3")
        .arg(runner)
        .arg(case_name)
        .arg(library_path)
        .output()
        .expect("run official LLVM dynamic runner");
    RuntimeOutput {
        status_code: output.status.code(),
        stdout: String::from_utf8(output.stdout).expect("runtime stdout should be UTF-8"),
        stderr: String::from_utf8(output.stderr).expect("runtime stderr should be UTF-8"),
    }
}

fn compile_and_run_llvm_object(
    object_path: &std::path::Path,
    harness_path: &std::path::Path,
    executable_path: &std::path::Path,
) -> String {
    let compile = Command::new("clang")
        .arg("-std=c11")
        .arg("-O3")
        .arg("-Wall")
        .arg("-Wextra")
        .arg("-Werror")
        .arg(object_path)
        .arg(harness_path)
        .arg("-o")
        .arg(executable_path)
        .output()
        .expect("run clang for LLVM object harness");
    assert!(
        compile.status.success(),
        "clang failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&compile.stdout),
        String::from_utf8_lossy(&compile.stderr)
    );

    let run = Command::new(executable_path)
        .output()
        .expect("run LLVM object harness");
    assert!(
        run.status.success(),
        "harness failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&run.stderr), "");
    String::from_utf8(run.stdout).expect("runtime stdout should be UTF-8")
}

fn pricing_llvm_object_harness() -> &'static str {
    r#"
#include <stdint.h>
#include <stdio.h>

typedef struct Item {
  int64_t price;
  int64_t qty;
  int64_t discount;
  int64_t tax_rate_ppm;
} Item;

int32_t calc_items(Item* items, int32_t len, int64_t* out);

int main(void) {
  Item items[3] = {
    { .price = 10000, .qty = 2, .discount = 1000, .tax_rate_ppm = 82500 },
    { .price = 2500, .qty = 4, .discount = 0, .tax_rate_ppm = 100000 },
    { .price = 1200, .qty = 5, .discount = 500, .tax_rate_ppm = 100000 }
  };
  int64_t out[3] = {0, 0, 0};
  int32_t status = calc_items(items, 3, out);
  printf("pricing-llvm-object:status=%d;out=%lld,%lld,%lld\n",
    status,
    (long long)out[0],
    (long long)out[1],
    (long long)out[2]);
  return 0;
}
"#
}

fn official_llvm_object_harness(case_name: &str) -> &'static str {
    match case_name {
        "llvm-scalar" => {
            r#"
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>

int64_t add_i64(int64_t a, int64_t b);
int32_t mul_i32(int32_t a, int32_t b);
bool less_i64(int64_t a, int64_t b);
uint64_t div_u64(uint64_t a, uint64_t b);

int main(void) {
  int64_t sum = add_i64(1, 2);
  int32_t product = mul_i32(3, 4);
  bool less = less_i64(1, 2);
  uint64_t quotient = div_u64(10, 2);
  if (sum != 3 || product != 12 || !less || quotient != 5) {
    return 1;
  }
  printf("llvm-scalar:add_i64=%lld;mul_i32=%d;less_i64=%d;div_u64=%llu\n",
    (long long)sum,
    product,
    less ? 1 : 0,
    (unsigned long long)quotient);
  return 0;
}
"#
        }
        "llvm-calls" => {
            r#"
#include <stdint.h>
#include <stdio.h>

int64_t calc(int64_t a, int64_t b);

int main(void) {
  int64_t result = calc(1, 2);
  if (result != 6) {
    return 1;
  }
  printf("llvm-calls:calc=%lld\n", (long long)result);
  return 0;
}
"#
        }
        "llvm-control-flow" => {
            r#"
#include <stdint.h>
#include <stdio.h>

int32_t max_i32(int32_t a, int32_t b);
int64_t sum_to_n(int64_t n);

int main(void) {
  int32_t high = max_i32(10, 3);
  int32_t low = max_i32(1, 3);
  int64_t sum = sum_to_n(5);
  if (high != 10 || low != 3 || sum != 10) {
    return 1;
  }
  printf("llvm-control-flow:max=%d,%d;sum=%lld\n", high, low, (long long)sum);
  return 0;
}
"#
        }
        "llvm-memory" => {
            r#"
#include <stdint.h>
#include <stdio.h>

typedef struct Item {
  int64_t price;
  int64_t qty;
  int64_t discount;
  int64_t tax_rate_ppm;
} Item;

int64_t first_price(Item* items);
int64_t get_price(Item* items, int32_t i);
int32_t write_i64(int64_t* out, int64_t value);

int main(void) {
  Item items[2] = {
    {100, 2, 5, 100000},
    {250, 3, 10, 200000},
  };
  int64_t out[1] = {0};
  int64_t first = first_price(items);
  int64_t indexed = get_price(items, 1);
  int32_t status = write_i64(out, 12345);
  if (first != 100 || indexed != 250 || status != 0 || out[0] != 12345) {
    return 1;
  }
  printf("llvm-memory:first=%lld;indexed=%lld;status=%d;stored=%lld\n",
    (long long)first,
    (long long)indexed,
    status,
    (long long)out[0]);
  return 0;
}
"#
        }
        "llvm-short-circuit" => {
            r#"
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>

bool and_short_circuit(int64_t a, int64_t b);
bool or_short_circuit(int64_t a, int64_t b);

int main(void) {
  bool values[4] = {
    and_short_circuit(0, 10),
    and_short_circuit(2, 10),
    or_short_circuit(0, 10),
    or_short_circuit(2, 10),
  };
  if (values[0] != false || values[1] != true || values[2] != true || values[3] != true) {
    return 1;
  }
  printf("llvm-short-circuit:out=%d,%d,%d,%d\n",
    values[0] ? 1 : 0,
    values[1] ? 1 : 0,
    values[2] ? 1 : 0,
    values[3] ? 1 : 0);
  return 0;
}
"#
        }
        "llvm-bool" => {
            r#"
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>

bool not_bool(bool a);
bool bool_local(bool a);
int32_t choose_bool(bool a, int32_t x, int32_t y);

int main(void) {
  bool not_true = not_bool(true);
  bool not_false = not_bool(false);
  bool local_true = bool_local(true);
  bool local_false = bool_local(false);
  int32_t choose_true = choose_bool(true, 10, 20);
  int32_t choose_false = choose_bool(false, 10, 20);
  if (
    not_true != false ||
    not_false != true ||
    local_true != false ||
    local_false != true ||
    choose_true != 10 ||
    choose_false != 20
  ) {
    return 1;
  }
  printf("llvm-bool:not=%d,%d;local=%d,%d;choose=%d,%d\n",
    not_true ? 1 : 0,
    not_false ? 1 : 0,
    local_true ? 1 : 0,
    local_false ? 1 : 0,
    choose_true,
    choose_false);
  return 0;
}
"#
        }
        "llvm-f64-edges" => f64_edges_llvm_object_harness(),
        "llvm-perf-f64-kernels" => f64_kernels_llvm_object_harness(),
        "llvm-f64-array" => f64_array_llvm_object_harness(),
        "llvm-f64-axpy" => f64_axpy_llvm_object_harness(),
        "llvm-f64-sum" => f64_sum_llvm_object_harness(),
        "llvm-pricing-soa" => pricing_soa_llvm_object_harness(),
        "llvm-dijkstra" => dijkstra_llvm_object_harness(),
        _ => panic!("missing official LLVM object harness for {case_name}"),
    }
}

fn f64_array_llvm_object_harness() -> &'static str {
    r#"
#include <math.h>
#include <stdint.h>
#include <stdio.h>

double axpy_f64(double a, double* x, double* y, int32_t len);

static int close_f64(double actual, double expected) {
  double diff = fabs(actual - expected);
  double scale = fmax(fmax(fabs(actual), fabs(expected)), 1.0);
  return diff <= 0.000000000001 * scale || diff <= 0.000000000001;
}

int main(void) {
  double x[4] = {1.0, 2.0, 3.0, 4.0};
  double y[4] = {0.5, 1.25, 1.25, 2.0};
  double expected[4] = {1.75, 3.75, 5.0, 7.0};
  double checksum = axpy_f64(1.25, x, y, 4);
  double expected_checksum = 17.5;

  if (!close_f64(checksum, expected_checksum)) {
    return 1;
  }
  for (int32_t i = 0; i < 4; i += 1) {
    if (!close_f64(y[i], expected[i])) {
      return 2;
    }
  }

  printf("llvm-f64-array:checksum=%.17g;out=%.17g,%.17g,%.17g,%.17g\n",
    checksum,
    y[0],
    y[1],
    y[2],
    y[3]);
  return 0;
}
"#
}

fn f64_axpy_llvm_object_harness() -> &'static str {
    r#"
#include <math.h>
#include <stdint.h>
#include <stdio.h>

double axpy_f64(double a, double* x, double* y, int32_t len);

static int close_f64(double actual, double expected) {
  double diff = fabs(actual - expected);
  double scale = fmax(fmax(fabs(actual), fabs(expected)), 1.0);
  return diff <= 0.000000000001 * scale || diff <= 0.000000000001;
}

int main(void) {
  double x[4] = {1.0, 2.0, 3.0, 4.0};
  double y[4] = {0.5, -1.0, 10.0, 20.0};
  double expected[4] = {2.5, 3.0, 16.0, 28.0};
  double checksum = axpy_f64(2.0, x, y, 4);
  double expected_checksum = 49.5;

  if (!close_f64(checksum, expected_checksum)) {
    return 1;
  }
  for (int32_t i = 0; i < 4; i += 1) {
    if (!close_f64(y[i], expected[i])) {
      return 2;
    }
  }

  printf("llvm-f64-axpy:checksum=%.17g;out=%.17g,%.17g,%.17g,%.17g\n",
    checksum,
    y[0],
    y[1],
    y[2],
    y[3]);
  return 0;
}
"#
}

fn f64_sum_llvm_object_harness() -> &'static str {
    r#"
#include <math.h>
#include <stdint.h>
#include <stdio.h>

double sum_f64(double* values, int32_t len);

static int close_f64(double actual, double expected) {
  double diff = fabs(actual - expected);
  double scale = fmax(fmax(fabs(actual), fabs(expected)), 1.0);
  return diff <= 0.000000000001 * scale || diff <= 0.000000000001;
}

int main(void) {
  double values[5] = {1.25, -2.5, 3.75, 4.5, 10.0};
  double result = sum_f64(values, 5);

  if (!close_f64(result, 17.0)) {
    return 1;
  }

  printf("llvm-f64-sum:result=%.17g;inputLength=5\n", result);
  return 0;
}
"#
}

fn pricing_soa_llvm_object_harness() -> &'static str {
    r#"
#include <stdint.h>
#include <stdio.h>

int32_t pricing_soa(
  int64_t* prices,
  int64_t* quantities,
  int64_t* discounts,
  int64_t* tax_rates_ppm,
  int64_t* out_totals,
  int32_t n
);

static int64_t expected_total(
  int64_t price,
  int64_t quantity,
  int64_t discount,
  int64_t tax_rate_ppm
) {
  int64_t after_discount = price * quantity - discount;
  int64_t tax = after_discount * tax_rate_ppm / 1000000;
  return after_discount + tax;
}

int main(void) {
  int64_t prices[4] = {10000, 2500, 1200, 999};
  int64_t quantities[4] = {2, 4, 5, 3};
  int64_t discounts[4] = {1000, 0, 500, 100};
  int64_t tax_rates[4] = {82500, 100000, 100000, 62500};
  int64_t out[4] = {0, 0, 0, 0};
  int32_t status = pricing_soa(prices, quantities, discounts, tax_rates, out, 4);
  int64_t expected[4] = {
    expected_total(prices[0], quantities[0], discounts[0], tax_rates[0]),
    expected_total(prices[1], quantities[1], discounts[1], tax_rates[1]),
    expected_total(prices[2], quantities[2], discounts[2], tax_rates[2]),
    expected_total(prices[3], quantities[3], discounts[3], tax_rates[3]),
  };

  if (status != 0) {
    return 1;
  }
  for (int32_t i = 0; i < 4; i += 1) {
    if (out[i] != expected[i]) {
      return 2;
    }
  }

  printf("llvm-pricing-soa:status=%d;out=%lld,%lld,%lld,%lld\n",
    status,
    (long long)out[0],
    (long long)out[1],
    (long long)out[2],
    (long long)out[3]);
  return 0;
}
"#
}

fn dijkstra_llvm_object_harness() -> &'static str {
    r#"
#include <stdint.h>
#include <stdio.h>

typedef struct DijkstraConfig {
  int32_t node_count;
  int32_t source;
  int64_t inf;
} DijkstraConfig;

int32_t dijkstra_matrix(
  DijkstraConfig* configs,
  int64_t* weights,
  int64_t* dist_out,
  int32_t* prev_out,
  int32_t* visited
);

int main(void) {
  DijkstraConfig config = {5, 0, 1000000};
  int64_t weights[25] = {
    0, 2, 5, 0, 0,
    0, 0, 1, 2, 9,
    0, 0, 0, 1, 0,
    0, 0, 0, 0, 3,
    0, 0, 0, 0, 0,
  };
  int64_t dist[5] = {0, 0, 0, 0, 0};
  int32_t prev[5] = {0, 0, 0, 0, 0};
  int32_t visited[5] = {0, 0, 0, 0, 0};
  int32_t settled = dijkstra_matrix(&config, weights, dist, prev, visited);
  int64_t expected_dist[5] = {0, 2, 3, 4, 7};
  int32_t expected_prev[5] = {0, 0, 1, 1, 3};
  int32_t expected_visited[5] = {1, 1, 1, 1, 1};

  if (settled != 5) {
    return 1;
  }
  for (int32_t i = 0; i < 5; i += 1) {
    if (dist[i] != expected_dist[i] || prev[i] != expected_prev[i] || visited[i] != expected_visited[i]) {
      return 2;
    }
  }
  printf("llvm-dijkstra:settled=%d;dist=%lld,%lld,%lld,%lld,%lld;prev=%d,%d,%d,%d,%d;visited=%d,%d,%d,%d,%d\n",
    settled,
    (long long)dist[0],
    (long long)dist[1],
    (long long)dist[2],
    (long long)dist[3],
    (long long)dist[4],
    prev[0],
    prev[1],
    prev[2],
    prev[3],
    prev[4],
    visited[0],
    visited[1],
    visited[2],
    visited[3],
    visited[4]);
  return 0;
}
"#
}

fn f64_kernels_llvm_object_harness() -> &'static str {
    r#"
#include <math.h>
#include <stdint.h>
#include <stdio.h>

double axpy_f64(double a, double* x, double* y, int32_t len);
double dot_f64(double* x, double* y, int32_t len);
double sum_f64(double* x, int32_t len);
double scale_f64(double a, double* x, int32_t len);

static int close_f64(double actual, double expected) {
  double diff = fabs(actual - expected);
  double scale = fabs(actual);
  double expected_abs = fabs(expected);
  if (expected_abs > scale) {
    scale = expected_abs;
  }
  if (scale < 1.0) {
    scale = 1.0;
  }
  return diff <= 0.0000001 || diff <= 0.00000001 * scale;
}

static int close_array(const double* actual, const double* expected, int32_t len) {
  for (int32_t index = 0; index < len; index += 1) {
    if (!close_f64(actual[index], expected[index])) {
      return 0;
    }
  }
  return 1;
}

int main(void) {
  const int32_t len = 4;
  const double x_input[4] = {1.0, -2.0, 3.5, 4.25};
  const double y_input[4] = {0.5, 8.0, -1.5, 2.25};
  double x[4] = {1.0, -2.0, 3.5, 4.25};
  double y[4] = {0.5, 8.0, -1.5, 2.25};
  double axpy_expected[4] = {2.0, 5.0, 3.75, 8.625};
  double axpy_checksum = axpy_f64(1.5, x, y, len);
  if (!close_f64(axpy_checksum, 19.375) || !close_array(y, axpy_expected, len)) {
    return 1;
  }

  double dot = dot_f64((double*)x_input, (double*)y_input, len);
  if (!close_f64(dot, -11.1875)) {
    return 2;
  }

  double sum = sum_f64((double*)x_input, len);
  if (!close_f64(sum, 6.75)) {
    return 3;
  }

  double scale_values[4] = {0.25, -1.5, 2.0, 10.0};
  double scale_expected[4] = {-0.5, 3.0, -4.0, -20.0};
  double scale = scale_f64(-2.0, scale_values, len);
  if (!close_f64(scale, -21.5) || !close_array(scale_values, scale_expected, len)) {
    return 4;
  }

  printf(
    "llvm-perf-f64-kernels:axpy=%.17g;dot=%.17g;sum=%.17g;scale=%.17g;"
    "axpyOut=%.17g,%.17g,%.17g,%.17g;scaleOut=%.17g,%.17g,%.17g,%.17g\n",
    axpy_checksum,
    dot,
    sum,
    scale,
    y[0],
    y[1],
    y[2],
    y[3],
    scale_values[0],
    scale_values[1],
    scale_values[2],
    scale_values[3]);
  return 0;
}
"#
}

fn f64_edges_llvm_object_harness() -> &'static str {
    r#"
#include <math.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>

typedef struct Quote {
  double price;
  double tax;
} Quote;

typedef struct NestedQuote {
  Quote quote;
  double fee;
} NestedQuote;

double finite_add(void);
double finite_sub(void);
double finite_mul(void);
double finite_div(void);
double tolerance_calc(void);
bool finite_less(void);
bool finite_less_equal(void);
bool finite_equal(void);
double negative_infinity(void);
double positive_infinity(void);
double not_a_number(void);
double negative_zero(void);
bool zero_equals_negative_zero(void);
bool nan_equals_nan(void);
bool nan_not_equals_nan(void);
bool nan_less_than_one(void);
bool nan_less_equal_one(void);
bool nan_greater_than_one(void);
bool nan_greater_equal_one(void);
double infinity_plus_finite(void);
double infinity_minus_infinity(void);
double overflow_to_infinity(void);
double underflow_smoke(void);
bool infinity_greater_than_finite(void);
bool negative_infinity_less_than_finite(void);
double ptr_read(double* values, int32_t index);
double ptr_write(double* values, int32_t index, double value);
double struct_read(Quote* quotes, int32_t index);
double struct_write(Quote* quotes, int32_t index, double value);
double nested_struct_read(NestedQuote* nested, int32_t index);
double nested_struct_write(NestedQuote* nested, int32_t index, double value);

static const char* class_f64(double value) {
  if (isnan(value)) {
    return "nan";
  }
  if (isinf(value)) {
    return signbit(value) ? "-inf" : "+inf";
  }
  if (value == 0.0 && signbit(value)) {
    return "-0";
  }
  if (value == 0.0) {
    return "+0";
  }
  return "finite";
}

static const char* ok(int pass) {
  return pass ? "ok" : "fail";
}

static int class_is(double value, const char* expected) {
  return strcmp(class_f64(value), expected) == 0;
}

static int close_f64(double actual, double expected) {
  double diff = fabs(actual - expected);
  double scale = fabs(actual);
  double expected_abs = fabs(expected);
  if (expected_abs > scale) {
    scale = expected_abs;
  }
  if (scale < 1.0) {
    scale = 1.0;
  }
  return diff <= 0.000000000001 * scale || diff <= 0.000000000001;
}

int main(void) {
  double values[3] = {1.0, 2.5, 4.0};
  Quote quotes[2] = {
    { .price = 10.25, .tax = 0.75 },
    { .price = 20.5, .tax = 1.25 }
  };
  NestedQuote nested[2] = {
    { .quote = { .price = 1.25, .tax = 0.75 }, .fee = 2.0 },
    { .quote = { .price = 10.0, .tax = 2.0 }, .fee = 3.0 }
  };
  double ptr_store_value = ptr_write(values, 1, 8.75);
  double struct_write_value = struct_write(quotes, 1, 0.5);
  double nested_write_value = nested_struct_write(nested, 1, 1.5);

  printf("llvm-f64-edges:");
  printf("finite_add=%s;", ok(close_f64(finite_add(), 4.0)));
  printf("finite_sub=%s;", ok(close_f64(finite_sub(), 3.5)));
  printf("finite_mul=%s;", ok(close_f64(finite_mul(), 3.75)));
  printf("finite_div=%s;", ok(close_f64(finite_div(), 3.5)));
  printf("tolerance_calc=%s;", ok(close_f64(tolerance_calc(), 10.0)));
  printf("finite_less=%s;", ok(finite_less() == true));
  printf("finite_less_equal=%s;", ok(finite_less_equal() == true));
  printf("finite_equal=%s;", ok(finite_equal() == true));
  printf("pos_inf=%s;", ok(class_is(positive_infinity(), "+inf")));
  printf("neg_inf=%s;", ok(class_is(negative_infinity(), "-inf")));
  printf("nan=%s;", ok(class_is(not_a_number(), "nan")));
  printf("neg_zero=%s;", ok(class_is(negative_zero(), "-0")));
  printf("zero_eq_neg_zero=%s;", ok(zero_equals_negative_zero() == true));
  printf("nan_eq_nan=%s;", ok(nan_equals_nan() == false));
  printf("nan_ne_nan=%s;", ok(nan_not_equals_nan() == true));
  printf("nan_lt_one=%s;", ok(nan_less_than_one() == false));
  printf("nan_le_one=%s;", ok(nan_less_equal_one() == false));
  printf("nan_gt_one=%s;", ok(nan_greater_than_one() == false));
  printf("nan_ge_one=%s;", ok(nan_greater_equal_one() == false));
  printf("inf_plus=%s;", ok(class_is(infinity_plus_finite(), "+inf")));
  printf("inf_minus_inf=%s;", ok(class_is(infinity_minus_infinity(), "nan")));
  printf("overflow=%s;", ok(class_is(overflow_to_infinity(), "+inf")));
  printf("underflow=%s;", ok(class_is(underflow_smoke(), "+0")));
  printf("inf_gt_finite=%s;", ok(infinity_greater_than_finite() == true));
  printf("neg_inf_lt_finite=%s;", ok(negative_infinity_less_than_finite() == true));
  printf("ptr_load=%s;", ok(close_f64(ptr_read(values, 2), 4.0)));
  printf("ptr_store=%s;", ok(close_f64(ptr_store_value, 8.75) && close_f64(values[1], 8.75)));
  printf("struct_read=%s;", ok(close_f64(struct_read(quotes, 0), 11.0)));
  printf("struct_write=%s;", ok(close_f64(struct_write_value, 21.0) && close_f64(quotes[1].tax, 0.5)));
  printf("nested_struct_read=%s;", ok(close_f64(nested_struct_read(nested, 0), 4.0)));
  printf("nested_struct_write=%s\n", ok(close_f64(nested_write_value, 14.5) && close_f64(nested[1].quote.tax, 1.5)));
  return 0;
}
"#
}

fn pricing_llvm_dynamic_runner() -> &'static str {
    r#"
from __future__ import annotations

import ctypes
import math
import sys


class Item(ctypes.Structure):
    _fields_ = [
        ("price", ctypes.c_int64),
        ("qty", ctypes.c_int64),
        ("discount", ctypes.c_int64),
        ("tax_rate_ppm", ctypes.c_int64),
    ]


def main() -> None:
    library_path = sys.argv[1]
    lib = ctypes.CDLL(library_path)
    lib.calc_items.argtypes = [
        ctypes.POINTER(Item),
        ctypes.c_int32,
        ctypes.POINTER(ctypes.c_int64),
    ]
    lib.calc_items.restype = ctypes.c_int32

    items = (Item * 3)(
        Item(price=10000, qty=2, discount=1000, tax_rate_ppm=82500),
        Item(price=2500, qty=4, discount=0, tax_rate_ppm=100000),
        Item(price=1200, qty=5, discount=500, tax_rate_ppm=100000),
    )
    out = (ctypes.c_int64 * len(items))(0, 0, 0)
    status = lib.calc_items(items, ctypes.c_int32(len(items)), out)
    expected = [20567, 11000, 6050]
    actual = list(out)
    if status != 0 or actual != expected:
        raise AssertionError(f"pricing LLVM dynamic mismatch status={status} actual={actual} expected={expected}")
    print(f"pricing-llvm-dynamic:status={status};out={','.join(str(value) for value in actual)}")


if __name__ == "__main__":
    main()
"#
}

fn official_llvm_dynamic_runner() -> &'static str {
    r#"
from __future__ import annotations

import ctypes
import math
import sys


class Item(ctypes.Structure):
    _fields_ = [
        ("price", ctypes.c_int64),
        ("qty", ctypes.c_int64),
        ("discount", ctypes.c_int64),
        ("tax_rate_ppm", ctypes.c_int64),
    ]


class Quote(ctypes.Structure):
    _fields_ = [
        ("price", ctypes.c_double),
        ("tax", ctypes.c_double),
    ]


class NestedQuote(ctypes.Structure):
    _fields_ = [
        ("quote", Quote),
        ("fee", ctypes.c_double),
    ]


class DijkstraConfig(ctypes.Structure):
    _fields_ = [
        ("node_count", ctypes.c_int32),
        ("source", ctypes.c_int32),
        ("inf", ctypes.c_int64),
    ]


def run_scalar(lib: ctypes.CDLL) -> str:
    lib.add_i64.argtypes = [ctypes.c_int64, ctypes.c_int64]
    lib.add_i64.restype = ctypes.c_int64
    lib.mul_i32.argtypes = [ctypes.c_int32, ctypes.c_int32]
    lib.mul_i32.restype = ctypes.c_int32
    lib.less_i64.argtypes = [ctypes.c_int64, ctypes.c_int64]
    lib.less_i64.restype = ctypes.c_bool
    lib.div_u64.argtypes = [ctypes.c_uint64, ctypes.c_uint64]
    lib.div_u64.restype = ctypes.c_uint64

    add_i64 = lib.add_i64(1, 2)
    mul_i32 = lib.mul_i32(3, 4)
    less_i64 = lib.less_i64(1, 2)
    div_u64 = lib.div_u64(10, 2)
    if add_i64 != 3 or mul_i32 != 12 or less_i64 is not True or div_u64 != 5:
        raise AssertionError(
            f"llvm-scalar mismatch add_i64={add_i64} mul_i32={mul_i32} "
            f"less_i64={less_i64} div_u64={div_u64}"
        )
    return f"llvm-scalar:add_i64={add_i64};mul_i32={mul_i32};less_i64={int(less_i64)};div_u64={div_u64}"


def run_calls(lib: ctypes.CDLL) -> str:
    lib.calc.argtypes = [ctypes.c_int64, ctypes.c_int64]
    lib.calc.restype = ctypes.c_int64
    result = lib.calc(1, 2)
    if result != 6:
        raise AssertionError(f"llvm-calls mismatch result={result}")
    return f"llvm-calls:calc={result}"


def run_control_flow(lib: ctypes.CDLL) -> str:
    lib.max_i32.argtypes = [ctypes.c_int32, ctypes.c_int32]
    lib.max_i32.restype = ctypes.c_int32
    lib.sum_to_n.argtypes = [ctypes.c_int64]
    lib.sum_to_n.restype = ctypes.c_int64
    high = lib.max_i32(10, 3)
    low = lib.max_i32(1, 3)
    total = lib.sum_to_n(5)
    if high != 10 or low != 3 or total != 10:
        raise AssertionError(f"llvm-control-flow mismatch high={high} low={low} total={total}")
    return f"llvm-control-flow:max={high},{low};sum={total}"


def run_memory(lib: ctypes.CDLL) -> str:
    lib.first_price.argtypes = [ctypes.POINTER(Item)]
    lib.first_price.restype = ctypes.c_int64
    lib.get_price.argtypes = [ctypes.POINTER(Item), ctypes.c_int32]
    lib.get_price.restype = ctypes.c_int64
    lib.write_i64.argtypes = [ctypes.POINTER(ctypes.c_int64), ctypes.c_int64]
    lib.write_i64.restype = ctypes.c_int32

    items = (Item * 2)(
        Item(price=100, qty=2, discount=5, tax_rate_ppm=100000),
        Item(price=250, qty=3, discount=10, tax_rate_ppm=200000),
    )
    out = (ctypes.c_int64 * 1)(0)
    first = lib.first_price(items)
    indexed = lib.get_price(items, ctypes.c_int32(1))
    status = lib.write_i64(out, ctypes.c_int64(12345))
    stored = out[0]
    if first != 100 or indexed != 250 or status != 0 or stored != 12345:
        raise AssertionError(
            f"llvm-memory mismatch first={first} indexed={indexed} "
            f"status={status} stored={stored}"
        )
    return f"llvm-memory:first={first};indexed={indexed};status={status};stored={stored}"


def run_short_circuit(lib: ctypes.CDLL) -> str:
    lib.and_short_circuit.argtypes = [ctypes.c_int64, ctypes.c_int64]
    lib.and_short_circuit.restype = ctypes.c_bool
    lib.or_short_circuit.argtypes = [ctypes.c_int64, ctypes.c_int64]
    lib.or_short_circuit.restype = ctypes.c_bool
    values = [
        lib.and_short_circuit(0, 10),
        lib.and_short_circuit(2, 10),
        lib.or_short_circuit(0, 10),
        lib.or_short_circuit(2, 10),
    ]
    expected = [False, True, True, True]
    if values != expected:
        raise AssertionError(f"llvm-short-circuit mismatch values={values} expected={expected}")
    encoded = ",".join(str(int(value)) for value in values)
    return f"llvm-short-circuit:out={encoded}"


def run_bool(lib: ctypes.CDLL) -> str:
    lib.not_bool.argtypes = [ctypes.c_bool]
    lib.not_bool.restype = ctypes.c_bool
    lib.bool_local.argtypes = [ctypes.c_bool]
    lib.bool_local.restype = ctypes.c_bool
    lib.choose_bool.argtypes = [ctypes.c_bool, ctypes.c_int32, ctypes.c_int32]
    lib.choose_bool.restype = ctypes.c_int32
    not_true = lib.not_bool(True)
    not_false = lib.not_bool(False)
    local_true = lib.bool_local(True)
    local_false = lib.bool_local(False)
    choose_true = lib.choose_bool(True, 10, 20)
    choose_false = lib.choose_bool(False, 10, 20)
    if (
        not_true is not False
        or not_false is not True
        or local_true is not False
        or local_false is not True
        or choose_true != 10
        or choose_false != 20
    ):
        raise AssertionError(
            "llvm-bool mismatch "
            f"not={not_true},{not_false} local={local_true},{local_false} "
            f"choose={choose_true},{choose_false}"
        )
    return (
        f"llvm-bool:not={int(not_true)},{int(not_false)};"
        f"local={int(local_true)},{int(local_false)};"
        f"choose={choose_true},{choose_false}"
    )


def run_f64_array(lib: ctypes.CDLL) -> str:
    double_ptr = ctypes.POINTER(ctypes.c_double)
    lib.axpy_f64.argtypes = [ctypes.c_double, double_ptr, double_ptr, ctypes.c_int32]
    lib.axpy_f64.restype = ctypes.c_double

    x_input = [1.0, 2.0, 3.0, 4.0]
    y_input = [0.5, 1.25, 1.25, 2.0]
    length = len(x_input)
    x = (ctypes.c_double * length)(*x_input)
    y = (ctypes.c_double * length)(*y_input)
    checksum = lib.axpy_f64(ctypes.c_double(1.25), x, y, ctypes.c_int32(length))
    actual = list(y)
    expected = [1.75, 3.75, 5.0, 7.0]
    expected_checksum = sum(expected)
    if not close_f64(checksum, expected_checksum) or any(
        not close_f64(value, expected[index]) for index, value in enumerate(actual)
    ):
        raise AssertionError(f"llvm-f64-array mismatch checksum={checksum} actual={actual}")
    return (
        f"llvm-f64-array:checksum={format_float(checksum)};"
        f"out={','.join(format_float(value) for value in actual)}"
    )


def run_f64_axpy(lib: ctypes.CDLL) -> str:
    double_ptr = ctypes.POINTER(ctypes.c_double)
    lib.axpy_f64.argtypes = [ctypes.c_double, double_ptr, double_ptr, ctypes.c_int32]
    lib.axpy_f64.restype = ctypes.c_double

    x_input = [1.0, 2.0, 3.0, 4.0]
    y_input = [0.5, -1.0, 10.0, 20.0]
    length = len(x_input)
    x = (ctypes.c_double * length)(*x_input)
    y = (ctypes.c_double * length)(*y_input)
    checksum = lib.axpy_f64(ctypes.c_double(2.0), x, y, ctypes.c_int32(length))
    actual = list(y)
    expected = [2.0 * value + y_input[index] for index, value in enumerate(x_input)]
    expected_checksum = sum(expected)
    if not close_f64(checksum, expected_checksum) or any(
        not close_f64(value, expected[index]) for index, value in enumerate(actual)
    ):
        raise AssertionError(f"llvm-f64-axpy mismatch checksum={checksum} actual={actual}")
    return (
        f"llvm-f64-axpy:checksum={format_float(checksum)};"
        f"out={','.join(format_float(value) for value in actual)}"
    )


def run_f64_sum(lib: ctypes.CDLL) -> str:
    double_ptr = ctypes.POINTER(ctypes.c_double)
    lib.sum_f64.argtypes = [double_ptr, ctypes.c_int32]
    lib.sum_f64.restype = ctypes.c_double

    input_values = [1.25, -2.5, 3.75, 4.5, 10.0]
    values = (ctypes.c_double * len(input_values))(*input_values)
    actual = lib.sum_f64(values, ctypes.c_int32(len(input_values)))
    expected = sum(input_values)
    if not close_f64(actual, expected):
        raise AssertionError(f"llvm-f64-sum mismatch actual={actual} expected={expected}")
    return f"llvm-f64-sum:result={format_float(actual)};inputLength={len(input_values)}"


def expected_total(price: int, quantity: int, discount: int, tax_rate_ppm: int) -> int:
    after_discount = price * quantity - discount
    tax = after_discount * tax_rate_ppm // 1000000
    return after_discount + tax


def run_pricing_soa(lib: ctypes.CDLL) -> str:
    int64_ptr = ctypes.POINTER(ctypes.c_int64)
    lib.pricing_soa.argtypes = [
        int64_ptr,
        int64_ptr,
        int64_ptr,
        int64_ptr,
        int64_ptr,
        ctypes.c_int32,
    ]
    lib.pricing_soa.restype = ctypes.c_int32

    rows = [
        (10000, 2, 1000, 82500),
        (2500, 4, 0, 100000),
        (1200, 5, 500, 100000),
        (999, 3, 100, 62500),
    ]
    length = len(rows)
    prices = (ctypes.c_int64 * length)(*(row[0] for row in rows))
    quantities = (ctypes.c_int64 * length)(*(row[1] for row in rows))
    discounts = (ctypes.c_int64 * length)(*(row[2] for row in rows))
    tax_rates = (ctypes.c_int64 * length)(*(row[3] for row in rows))
    out = (ctypes.c_int64 * length)()
    status = lib.pricing_soa(prices, quantities, discounts, tax_rates, out, ctypes.c_int32(length))
    actual = list(out)
    expected = [expected_total(*row) for row in rows]
    if status != 0 or actual != expected:
        raise AssertionError(f"llvm-pricing-soa mismatch status={status} actual={actual} expected={expected}")
    return f"llvm-pricing-soa:status={status};out={','.join(str(value) for value in actual)}"


def run_dijkstra(lib: ctypes.CDLL) -> str:
    lib.dijkstra_matrix.argtypes = [
        ctypes.POINTER(DijkstraConfig),
        ctypes.POINTER(ctypes.c_int64),
        ctypes.POINTER(ctypes.c_int64),
        ctypes.POINTER(ctypes.c_int32),
        ctypes.POINTER(ctypes.c_int32),
    ]
    lib.dijkstra_matrix.restype = ctypes.c_int32

    node_count = 5
    config = DijkstraConfig(node_count=node_count, source=0, inf=1_000_000)
    weights = (ctypes.c_int64 * (node_count * node_count))(
        0, 2, 5, 0, 0,
        0, 0, 1, 2, 9,
        0, 0, 0, 1, 0,
        0, 0, 0, 0, 3,
        0, 0, 0, 0, 0,
    )
    dist = (ctypes.c_int64 * node_count)()
    prev = (ctypes.c_int32 * node_count)()
    visited = (ctypes.c_int32 * node_count)()

    settled = lib.dijkstra_matrix(
        ctypes.byref(config),
        weights,
        dist,
        prev,
        visited,
    )
    actual_dist = list(dist)
    actual_prev = list(prev)
    actual_visited = list(visited)
    expected_dist = [0, 2, 3, 4, 7]
    expected_prev = [0, 0, 1, 1, 3]
    expected_visited = [1, 1, 1, 1, 1]
    if (
        settled != node_count
        or actual_dist != expected_dist
        or actual_prev != expected_prev
        or actual_visited != expected_visited
    ):
        raise AssertionError(
            "llvm-dijkstra mismatch "
            f"settled={settled} dist={actual_dist} prev={actual_prev} visited={actual_visited}"
        )
    return (
        f"llvm-dijkstra:settled={settled};"
        f"dist={','.join(str(value) for value in actual_dist)};"
        f"prev={','.join(str(value) for value in actual_prev)};"
        f"visited={','.join(str(value) for value in actual_visited)}"
    )


def classify_f64(value: float) -> str:
    if math.isnan(value):
        return "nan"
    if math.isinf(value):
        return "-inf" if math.copysign(1.0, value) < 0 else "+inf"
    if value == 0.0:
        return "-0" if math.copysign(1.0, value) < 0 else "+0"
    return "finite"


def class_is(value: float, expected: str) -> bool:
    return classify_f64(value) == expected


def close_f64(actual: float, expected: float) -> bool:
    diff = abs(actual - expected)
    scale = max(abs(actual), abs(expected), 1.0)
    return diff <= 0.000000000001 * scale or diff <= 0.000000000001


def ok(value: bool) -> str:
    return "ok" if value else "fail"


def format_float(value: float) -> str:
    return format(value, ".17g")


def run_f64_edges(lib: ctypes.CDLL) -> str:
    for name in [
        "finite_add",
        "finite_sub",
        "finite_mul",
        "finite_div",
        "tolerance_calc",
        "negative_infinity",
        "positive_infinity",
        "not_a_number",
        "negative_zero",
        "infinity_plus_finite",
        "infinity_minus_infinity",
        "overflow_to_infinity",
        "underflow_smoke",
    ]:
        func = getattr(lib, name)
        func.argtypes = []
        func.restype = ctypes.c_double

    for name in [
        "finite_less",
        "finite_less_equal",
        "finite_equal",
        "zero_equals_negative_zero",
        "nan_equals_nan",
        "nan_not_equals_nan",
        "nan_less_than_one",
        "nan_less_equal_one",
        "nan_greater_than_one",
        "nan_greater_equal_one",
        "infinity_greater_than_finite",
        "negative_infinity_less_than_finite",
    ]:
        func = getattr(lib, name)
        func.argtypes = []
        func.restype = ctypes.c_bool

    lib.ptr_read.argtypes = [ctypes.POINTER(ctypes.c_double), ctypes.c_int32]
    lib.ptr_read.restype = ctypes.c_double
    lib.ptr_write.argtypes = [ctypes.POINTER(ctypes.c_double), ctypes.c_int32, ctypes.c_double]
    lib.ptr_write.restype = ctypes.c_double
    lib.struct_read.argtypes = [ctypes.POINTER(Quote), ctypes.c_int32]
    lib.struct_read.restype = ctypes.c_double
    lib.struct_write.argtypes = [ctypes.POINTER(Quote), ctypes.c_int32, ctypes.c_double]
    lib.struct_write.restype = ctypes.c_double
    lib.nested_struct_read.argtypes = [ctypes.POINTER(NestedQuote), ctypes.c_int32]
    lib.nested_struct_read.restype = ctypes.c_double
    lib.nested_struct_write.argtypes = [ctypes.POINTER(NestedQuote), ctypes.c_int32, ctypes.c_double]
    lib.nested_struct_write.restype = ctypes.c_double

    values = (ctypes.c_double * 3)(1.0, 2.5, 4.0)
    quotes = (Quote * 2)(
        Quote(price=10.25, tax=0.75),
        Quote(price=20.5, tax=1.25),
    )
    nested = (NestedQuote * 2)(
        NestedQuote(quote=Quote(price=1.25, tax=0.75), fee=2.0),
        NestedQuote(quote=Quote(price=10.0, tax=2.0), fee=3.0),
    )
    ptr_store_value = lib.ptr_write(values, ctypes.c_int32(1), ctypes.c_double(8.75))
    struct_write_value = lib.struct_write(quotes, ctypes.c_int32(1), ctypes.c_double(0.5))
    nested_write_value = lib.nested_struct_write(nested, ctypes.c_int32(1), ctypes.c_double(1.5))

    checks = [
        ("finite_add", close_f64(lib.finite_add(), 4.0)),
        ("finite_sub", close_f64(lib.finite_sub(), 3.5)),
        ("finite_mul", close_f64(lib.finite_mul(), 3.75)),
        ("finite_div", close_f64(lib.finite_div(), 3.5)),
        ("tolerance_calc", close_f64(lib.tolerance_calc(), 10.0)),
        ("finite_less", lib.finite_less() is True),
        ("finite_less_equal", lib.finite_less_equal() is True),
        ("finite_equal", lib.finite_equal() is True),
        ("pos_inf", class_is(lib.positive_infinity(), "+inf")),
        ("neg_inf", class_is(lib.negative_infinity(), "-inf")),
        ("nan", class_is(lib.not_a_number(), "nan")),
        ("neg_zero", class_is(lib.negative_zero(), "-0")),
        ("zero_eq_neg_zero", lib.zero_equals_negative_zero() is True),
        ("nan_eq_nan", lib.nan_equals_nan() is False),
        ("nan_ne_nan", lib.nan_not_equals_nan() is True),
        ("nan_lt_one", lib.nan_less_than_one() is False),
        ("nan_le_one", lib.nan_less_equal_one() is False),
        ("nan_gt_one", lib.nan_greater_than_one() is False),
        ("nan_ge_one", lib.nan_greater_equal_one() is False),
        ("inf_plus", class_is(lib.infinity_plus_finite(), "+inf")),
        ("inf_minus_inf", class_is(lib.infinity_minus_infinity(), "nan")),
        ("overflow", class_is(lib.overflow_to_infinity(), "+inf")),
        ("underflow", class_is(lib.underflow_smoke(), "+0")),
        ("inf_gt_finite", lib.infinity_greater_than_finite() is True),
        ("neg_inf_lt_finite", lib.negative_infinity_less_than_finite() is True),
        ("ptr_load", close_f64(lib.ptr_read(values, ctypes.c_int32(2)), 4.0)),
        ("ptr_store", close_f64(ptr_store_value, 8.75) and close_f64(values[1], 8.75)),
        ("struct_read", close_f64(lib.struct_read(quotes, ctypes.c_int32(0)), 11.0)),
        ("struct_write", close_f64(struct_write_value, 21.0) and close_f64(quotes[1].tax, 0.5)),
        ("nested_struct_read", close_f64(lib.nested_struct_read(nested, ctypes.c_int32(0)), 4.0)),
        (
            "nested_struct_write",
            close_f64(nested_write_value, 14.5) and close_f64(nested[1].quote.tax, 1.5),
        ),
    ]
    return "llvm-f64-edges:" + ";".join(f"{name}={ok(passed)}" for name, passed in checks)


def run_f64_kernels(lib: ctypes.CDLL) -> str:
    double_ptr = ctypes.POINTER(ctypes.c_double)
    lib.axpy_f64.argtypes = [ctypes.c_double, double_ptr, double_ptr, ctypes.c_int32]
    lib.axpy_f64.restype = ctypes.c_double
    lib.dot_f64.argtypes = [double_ptr, double_ptr, ctypes.c_int32]
    lib.dot_f64.restype = ctypes.c_double
    lib.sum_f64.argtypes = [double_ptr, ctypes.c_int32]
    lib.sum_f64.restype = ctypes.c_double
    lib.scale_f64.argtypes = [ctypes.c_double, double_ptr, ctypes.c_int32]
    lib.scale_f64.restype = ctypes.c_double

    x_input = [1.0, -2.0, 3.5, 4.25]
    y_input = [0.5, 8.0, -1.5, 2.25]
    length = len(x_input)
    x = (ctypes.c_double * length)(*x_input)
    y = (ctypes.c_double * length)(*y_input)
    axpy_checksum = lib.axpy_f64(ctypes.c_double(1.5), x, y, ctypes.c_int32(length))
    axpy_actual = list(y)
    axpy_expected = [1.5 * value + y_input[index] for index, value in enumerate(x_input)]
    axpy_expected_checksum = sum(axpy_expected)
    if not close_f64(axpy_checksum, axpy_expected_checksum) or any(
        not close_f64(actual, expected) for actual, expected in zip(axpy_actual, axpy_expected)
    ):
        raise AssertionError(f"llvm-perf-f64-kernels axpy mismatch checksum={axpy_checksum} out={axpy_actual}")

    x = (ctypes.c_double * length)(*x_input)
    y = (ctypes.c_double * length)(*y_input)
    dot_actual = lib.dot_f64(x, y, ctypes.c_int32(length))
    dot_expected = sum(value * y_input[index] for index, value in enumerate(x_input))
    if not close_f64(dot_actual, dot_expected):
        raise AssertionError(f"llvm-perf-f64-kernels dot mismatch actual={dot_actual} expected={dot_expected}")

    sum_actual = lib.sum_f64(x, ctypes.c_int32(length))
    sum_expected = sum(x_input)
    if not close_f64(sum_actual, sum_expected):
        raise AssertionError(f"llvm-perf-f64-kernels sum mismatch actual={sum_actual} expected={sum_expected}")

    scale_input = [0.25, -1.5, 2.0, 10.0]
    scale = (ctypes.c_double * length)(*scale_input)
    scale_checksum = lib.scale_f64(ctypes.c_double(-2.0), scale, ctypes.c_int32(length))
    scale_actual = list(scale)
    scale_expected = [-2.0 * value for value in scale_input]
    scale_expected_checksum = sum(scale_expected)
    if not close_f64(scale_checksum, scale_expected_checksum) or any(
        not close_f64(actual, expected) for actual, expected in zip(scale_actual, scale_expected)
    ):
        raise AssertionError(f"llvm-perf-f64-kernels scale mismatch checksum={scale_checksum} out={scale_actual}")

    return (
        f"llvm-perf-f64-kernels:axpy={format_float(axpy_checksum)};"
        f"dot={format_float(dot_actual)};sum={format_float(sum_actual)};"
        f"scale={format_float(scale_checksum)};"
        f"axpyOut={','.join(format_float(value) for value in axpy_actual)};"
        f"scaleOut={','.join(format_float(value) for value in scale_actual)}"
    )


RUNNERS = {
    "llvm-scalar": run_scalar,
    "llvm-calls": run_calls,
    "llvm-control-flow": run_control_flow,
    "llvm-memory": run_memory,
    "llvm-short-circuit": run_short_circuit,
    "llvm-bool": run_bool,
    "llvm-dijkstra": run_dijkstra,
    "llvm-f64-array": run_f64_array,
    "llvm-f64-axpy": run_f64_axpy,
    "llvm-f64-sum": run_f64_sum,
    "llvm-pricing-soa": run_pricing_soa,
    "llvm-f64-edges": run_f64_edges,
    "llvm-perf-f64-kernels": run_f64_kernels,
}


def main() -> None:
    case_name = sys.argv[1]
    library_path = sys.argv[2]
    runner = RUNNERS.get(case_name)
    if runner is None:
        raise AssertionError(f"unknown LLVM dynamic case: {case_name}")
    lib = ctypes.CDLL(library_path)
    print(runner(lib))


if __name__ == "__main__":
    main()
"#
}
