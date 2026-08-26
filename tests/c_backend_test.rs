use std::{
    fs,
    path::PathBuf,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

use calckernel::{
    EmitCOptions, MirPassContext, MirPassOverflowMode, MirPassTargetBackend, OverflowMode,
    SourceFile, build_mir_optimization_pipeline, check, emit_c_module, lower_to_mir,
    run_mir_pass_pipeline,
};

fn emit_c(source_text: &str) -> String {
    emit_c_with_overflow(source_text, OverflowMode::Unchecked)
}

fn emit_checked_c(source_text: &str) -> String {
    emit_c_with_overflow(source_text, OverflowMode::Checked)
}

fn emit_c_with_overflow(source_text: &str, overflow_mode: OverflowMode) -> String {
    emit_c_with_overflow_and_opt_level(source_text, overflow_mode, 1)
}

fn emit_c_with_overflow_and_opt_level(
    source_text: &str,
    overflow_mode: OverflowMode,
    opt_level: u8,
) -> String {
    let checked = check(&SourceFile::new("test.ck", source_text));
    assert_eq!(checked.diagnostics, []);
    let mir = lower_to_mir(&checked.checked_program).expect("MIR lowering should succeed");
    let pipeline = build_mir_optimization_pipeline(opt_level);
    let optimized = run_mir_pass_pipeline(
        mir,
        &pipeline,
        &MirPassContext {
            opt_level,
            overflow_mode: match overflow_mode {
                OverflowMode::Unchecked => MirPassOverflowMode::Unchecked,
                OverflowMode::Checked => MirPassOverflowMode::Checked,
            },
            target_backend: MirPassTargetBackend::C,
            debug: Default::default(),
        },
    );
    assert_eq!(optimized.validation_errors, []);
    emit_c_module(
        &optimized.module,
        EmitCOptions {
            overflow_mode,
            opt_level,
        },
    )
}

#[test]
fn c_backend_should_compile_and_run_scalar_control_and_memory_program() {
    let c = emit_c(
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

    let harness = format!(
        r#"
{c}

int main(void) {{
  if (add_i64(2, 3) != 5) return 1;
  if (sum_to_n(5) != 10) return 2;
  Item items[1] = {{ {{ 7, 6 }} }};
  int64_t out[1] = {{0}};
  if (calc(items, out) != 0) return 3;
  if (out[0] != 42) return 4;
  if (as_f64(-2, 5) != 3.0) return 5;
  return 0;
}}
"#
    );

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("rust_calckernel_c_backend_{unique}"));
    fs::create_dir_all(&dir).expect("create temp dir");
    let c_path = dir.join("harness.c");
    let bin_path = dir.join("harness");
    fs::write(&c_path, harness).expect("write harness");

    let compile = Command::new("clang")
        .arg("-std=c11")
        .arg("-Wall")
        .arg("-Wextra")
        .arg("-Werror")
        .arg(&c_path)
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
fn checked_c_backend_should_compile_and_return_status_codes() {
    let c = emit_checked_c(
        r#"
      fn helper(a: i64, b: i64) -> i64 {
        return a + b;
      }

      export fn add_i64(a: i64, b: i64) -> i64 {
        return a + b;
      }

      export fn div_i64(a: i64, b: i64) -> i64 {
        return a / b;
      }

      export fn neg_i64(a: i64) -> i64 {
        return -a;
      }

      export fn call_helper(a: i64, b: i64) -> i64 {
        return helper(a, b) * 2;
      }
    "#,
    );

    assert!(c.contains("typedef int32_t CK_Status;"));
    assert!(c.contains("#define CK_OK ((CK_Status)0)"));
    assert!(c.contains("CK_Status add_i64(int64_t a, int64_t b, int64_t* ck_return)"));
    assert!(c.contains("__builtin_add_overflow"));

    let harness = format!(
        r#"
{c}

int main(void) {{
  int64_t value = 0;
  if (add_i64(2, 3, &value) != CK_OK || value != 5) return 1;
  if (add_i64(INT64_MAX, 1, &value) != CK_ERR_OVERFLOW) return 2;
  if (div_i64(10, 0, &value) != CK_ERR_DIV_BY_ZERO) return 3;
  if (div_i64(INT64_MIN, -1, &value) != CK_ERR_OVERFLOW) return 4;
  if (neg_i64(INT64_MIN, &value) != CK_ERR_OVERFLOW) return 5;
  if (add_i64(1, 2, 0) != CK_ERR_NULL_POINTER) return 6;
  if (call_helper(4, 5, &value) != CK_OK || value != 18) return 7;
  if (call_helper(INT64_MAX, 1, &value) != CK_ERR_OVERFLOW) return 8;
  return 0;
}}
"#
    );

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("rust_calckernel_checked_c_backend_{unique}"));
    fs::create_dir_all(&dir).expect("create temp dir");
    let c_path = dir.join("checked_harness.c");
    let bin_path = dir.join("checked_harness");
    fs::write(&c_path, harness).expect("write harness");

    let compile = Command::new("clang")
        .arg("-std=c11")
        .arg("-Wall")
        .arg("-Wextra")
        .arg("-Werror")
        .arg(&c_path)
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
fn checked_c_backend_should_remove_only_proven_safe_induction_overflow_checks_at_o3() {
    let source = r#"
      export fn fill(out: ptr<i64>, len: i32) -> i32 {
        let i: i32 = 0;
        while i < len {
          out[i] = 0;
          i = i + 1;
        }
        return 0;
      }
    "#;

    let o0 = emit_c_with_overflow_and_opt_level(source, OverflowMode::Checked, 0);
    let o3 = emit_c_with_overflow_and_opt_level(source, OverflowMode::Checked, 3);

    assert!(o0.contains("__builtin_add_overflow(i,"));
    assert!(!o3.contains("__builtin_add_overflow(i,"));
    assert!(o3.contains(" = i + "));
}

#[test]
fn c_backend_should_match_typescript_oracle_for_official_examples() {
    let Some(ts_cli) = typescript_cli() else {
        return;
    };
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("rust_calckernel_c_oracle_{unique}"));
    let ts_dir = dir.join("ts");
    let rust_dir = dir.join("rust");
    fs::create_dir_all(&ts_dir).expect("create TS temp dir");
    fs::create_dir_all(&rust_dir).expect("create Rust temp dir");
    let examples = [
        "examples/scalar.ck",
        "examples/explicit_casts.ck",
        "examples/pricing.ck",
        "examples/dijkstra.ck",
        "examples/scalar_checked.ck",
        "examples/scalar_control_checked.ck",
        "examples/scalar_calls_checked.ck",
        "examples/scalar_logical_checked.ck",
        "examples/llvm_scalar.ck",
        "examples/llvm_calls.ck",
        "examples/llvm_memory.ck",
        "examples/llvm_control_flow.ck",
        "examples/llvm_short_circuit.ck",
        "examples/llvm_bool.ck",
        "examples/wasm_scalar.ck",
        "examples/wasm_calls.ck",
        "examples/wasm_memory.ck",
        "examples/wasm_control_flow.ck",
        "examples/wasm_short_circuit.ck",
        "examples/node-wasm-f64-array/f64_array.ck",
        "examples/wasm/f64-axpy/axpy.ck",
        "examples/wasm/f64-sum/sum.ck",
        "examples/wasm/pricing-soa/pricing_soa.ck",
        "tests/fixtures/f64_edges.ck",
    ];

    for (index, example) in examples.iter().enumerate() {
        let source = typescript_root().join(example);
        let ts_out = ts_dir.join(format!("case_{index}")).join("out.c");
        let rust_out = rust_dir.join(format!("case_{index}")).join("out.c");

        let ts_output = Command::new("node")
            .arg(&ts_cli)
            .arg("emit-c")
            .arg("--out")
            .arg(&ts_out)
            .arg(&source)
            .output()
            .expect("run TypeScript emit-c");
        assert!(
            ts_output.status.success(),
            "{example} TS stderr:\n{}",
            String::from_utf8_lossy(&ts_output.stderr)
        );

        let rust_output = Command::new(env!("CARGO_BIN_EXE_ckc"))
            .arg("emit-c")
            .arg("--out")
            .arg(&rust_out)
            .arg(&source)
            .output()
            .expect("run Rust emit-c");
        assert!(
            rust_output.status.success(),
            "{example} Rust stderr:\n{}",
            String::from_utf8_lossy(&rust_output.stderr)
        );

        assert_eq!(
            fs::read_to_string(&rust_out).expect("read Rust C"),
            fs::read_to_string(&ts_out).expect("read TS C"),
            "{example} C output"
        );
        assert_eq!(
            fs::read_to_string(rust_out.with_extension("h")).expect("read Rust header"),
            fs::read_to_string(ts_out.with_extension("h")).expect("read TS header"),
            "{example} header output"
        );
    }
}

#[test]
fn c_backend_should_match_typescript_oracle_for_perf_fixtures_at_benchmark_opt_levels() {
    let Some(ts_cli) = typescript_cli() else {
        return;
    };
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("rust_calckernel_c_perf_oracle_{unique}"));
    fs::create_dir_all(&dir).expect("create temp dir");
    let cases = [
        (
            "pricing_helpers_o0",
            "bench/perf/fixtures/pricing_helpers.ck",
            "-O0",
        ),
        (
            "pricing_helpers_o2",
            "bench/perf/fixtures/pricing_helpers.ck",
            "-O2",
        ),
        (
            "pricing_soa_o3",
            "bench/perf/fixtures/pricing_soa.ck",
            "-O3",
        ),
        (
            "f64_kernels_o3",
            "bench/perf/fixtures/f64_kernels.ck",
            "-O3",
        ),
    ];

    for (case_name, fixture, opt_level) in cases {
        let source = typescript_root().join(fixture);
        let output_dir = dir.join(case_name);
        fs::create_dir_all(&output_dir).expect("create case temp dir");
        let out = output_dir.join("out.c");
        let header = out.with_extension("h");

        let ts_output = Command::new("node")
            .arg(&ts_cli)
            .arg("emit-c")
            .arg("--out")
            .arg(&out)
            .arg("--header")
            .arg(&header)
            .arg("--overflow")
            .arg("unchecked")
            .arg(opt_level)
            .arg(&source)
            .output()
            .expect("run TypeScript emit-c");
        assert!(
            ts_output.status.success(),
            "{case_name} TS stderr:\n{}",
            String::from_utf8_lossy(&ts_output.stderr)
        );
        let ts_stdout = String::from_utf8(ts_output.stdout).expect("TS stdout should be UTF-8");
        let ts_stderr = String::from_utf8(ts_output.stderr).expect("TS stderr should be UTF-8");
        let ts_c = fs::read_to_string(&out).expect("read TS C");
        let ts_h = fs::read_to_string(&header).expect("read TS header");

        let rust_output = Command::new(env!("CARGO_BIN_EXE_ckc"))
            .arg("emit-c")
            .arg("--out")
            .arg(&out)
            .arg("--header")
            .arg(&header)
            .arg("--overflow")
            .arg("unchecked")
            .arg(opt_level)
            .arg(&source)
            .output()
            .expect("run Rust emit-c");
        assert!(
            rust_output.status.success(),
            "{case_name} Rust stderr:\n{}",
            String::from_utf8_lossy(&rust_output.stderr)
        );

        assert_eq!(
            String::from_utf8(rust_output.stdout).expect("Rust stdout should be UTF-8"),
            ts_stdout,
            "{case_name} stdout"
        );
        assert_eq!(
            String::from_utf8(rust_output.stderr).expect("Rust stderr should be UTF-8"),
            ts_stderr,
            "{case_name} stderr"
        );
        assert_eq!(
            fs::read_to_string(&out).expect("read Rust C"),
            ts_c,
            "{case_name} C output"
        );
        assert_eq!(
            fs::read_to_string(header).expect("read Rust header"),
            ts_h,
            "{case_name} header output"
        );
    }
}

#[test]
fn c_backend_should_run_nested_break_continue_at_all_opt_levels() {
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

    for opt_level in 0..=3 {
        let c = emit_c_with_overflow_and_opt_level(source, OverflowMode::Unchecked, opt_level);
        let harness = format!(
            r#"
{c}

int main(void) {{
  if (early_exit(10) != 3) return 1;
  if (skip_three(5) != 12) return 2;
  if (nested(4) != 8) return 3;
  if (return_from_loop(5) != 99) return 4;
  return 0;
}}
"#
        );
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "rust_calckernel_break_continue_c_{unique}_o{opt_level}"
        ));
        fs::create_dir_all(&dir).expect("create temp dir");
        let c_path = dir.join("harness.c");
        let bin_path = dir.join("harness");
        fs::write(&c_path, harness).expect("write harness");

        let compile = Command::new("clang")
            .arg("-std=c11")
            .arg("-Wall")
            .arg("-Wextra")
            .arg("-Werror")
            .arg(&c_path)
            .arg("-o")
            .arg(&bin_path)
            .output()
            .expect("run clang");
        assert!(
            compile.status.success(),
            "O{opt_level} clang stderr:\n{}",
            String::from_utf8_lossy(&compile.stderr)
        );

        let run = Command::new(&bin_path).output().expect("run C harness");
        assert!(
            run.status.success(),
            "O{opt_level} runtime stderr:\n{}",
            String::from_utf8_lossy(&run.stderr)
        );
    }
}

fn typescript_cli() -> Option<PathBuf> {
    let cli = typescript_root().join("dist/src/cli.js");
    cli.exists().then_some(cli)
}

fn typescript_root() -> PathBuf {
    std::env::var_os("CALCKERNEL_TS_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/Users/lynn/code/CalcKernel"))
}
