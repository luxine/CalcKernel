use std::{path::PathBuf, process::Command};

use calckernel::{SourceFile, check, lower_to_mir, print_mir_module, validate_mir_module};

fn lower_and_print(source_text: &str) -> String {
    let checked = check(&SourceFile::new("test.ck", source_text));
    assert_eq!(checked.diagnostics, []);
    let mir = lower_to_mir(&checked.checked_program).expect("MIR lowering should succeed");
    assert_eq!(validate_mir_module(&mir).errors, []);
    print_mir_module(&mir)
}

#[test]
fn mir_should_lower_scalar_straight_line_functions() {
    assert_eq!(
        lower_and_print(
            r#"
        export fn add_i64(a: i64, b: i64) -> i64 {
          let x: i64 = a + b;
          return x;
        }

        export fn assign_i64(a: i64, b: i64) -> i64 {
          let x: i64 = a;
          x = b - 1;
          return x;
        }
      "#
        ),
        "export fn add_i64(a: i64, b: i64) -> i64 {
  local x: i64

bb0:
  %t0: i64 = add a, b
  x: i64 = move %t0
  return x
}

export fn assign_i64(a: i64, b: i64) -> i64 {
  local x: i64

bb0:
  x: i64 = move a
  %t0: i64 = const_int 1
  %t1: i64 = sub b, %t0
  x: i64 = move %t1
  return x
}
"
    );
}

#[test]
fn mir_should_lower_control_flow() {
    assert_eq!(
        lower_and_print(
            r#"
        export fn sum_to_n(n: i64) -> i64 {
          let i: i64 = 0;
          let sum: i64 = 0;

          while i < n {
            sum = sum + i;
            i = i + 1;
          }

          return sum;
        }
      "#
        ),
        "export fn sum_to_n(n: i64) -> i64 {
  local i: i64
  local sum: i64

bb0:
  %t0: i64 = const_int 0
  i: i64 = move %t0
  %t1: i64 = const_int 0
  sum: i64 = move %t1
  jump bb1

bb1:
  %t2: bool = lt i, n
  branch %t2, bb2, bb3

bb2:
  %t3: i64 = add sum, i
  sum: i64 = move %t3
  %t4: i64 = const_int 1
  %t5: i64 = add i, %t4
  i: i64 = move %t5
  jump bb1

bb3:
  return sum
}
"
    );
}

#[test]
fn mir_should_lower_places() {
    assert_eq!(
        lower_and_print(
            r#"
          struct Quote {
            price: f64;
            qty: i64;
          }

          export fn update(items: ptr<Quote>, out: ptr<f64>, i: i32) -> f64 {
            let price: f64 = items[i].price;
            out[i] = price;
            return out[i];
          }
        "#
        ),
        "struct Quote {
  price: f64
  qty: i64
}

export fn update(items: ptr<Quote>, out: ptr<f64>, i: i32) -> f64 {
  local price: f64

bb0:
  %t0: f64 = load field(index(items, i), price)
  price: f64 = move %t0
  store index(out, i), price
  %t1: f64 = load index(out, i)
  return %t1
}
"
    );
}

#[test]
fn mir_should_lower_short_circuit_logical_operators() {
    assert_eq!(
        lower_and_print(
            r#"
        export fn and_short_circuit(a: i64, b: i64) -> bool {
          return a != 0 && b / a > 1;
        }
      "#
        ),
        "export fn and_short_circuit(a: i64, b: i64) -> bool {
  local ik_sc0: bool

bb0:
  %t0: i64 = const_int 0
  %t1: bool = ne a, %t0
  branch %t1, bb1, bb2

bb1:
  %t2: i64 = div b, a
  %t3: i64 = const_int 1
  %t4: bool = gt %t2, %t3
  ik_sc0: bool = move %t4
  jump bb3

bb2:
  ik_sc0: bool = move false
  jump bb3

bb3:
  return ik_sc0
}
"
    );
}

#[test]
fn mir_should_lower_break_to_innermost_loop_exit() {
    let text = lower_and_print(
        r#"
      export fn stop_at_three(n: u32) -> u32 {
        let i: u32 = 0;
        while i < n {
          if i == 3 {
            break;
          }
          i = i + 1;
        }
        return i;
      }
    "#,
    );

    assert!(text.contains("bb4:\n  jump bb3\n"), "{text}");
}

#[test]
fn mir_should_lower_continue_to_innermost_loop_condition() {
    let text = lower_and_print(
        r#"
      export fn skip_three(n: u32) -> u32 {
        let i: u32 = 0;
        let total: u32 = 0;
        while i < n {
          i = i + 1;
          if i == 3 {
            continue;
          }
          total = total + i;
        }
        return total;
      }
    "#,
    );

    assert!(text.contains("bb4:\n  jump bb1\n"), "{text}");
}

#[test]
fn mir_should_lower_nested_loop_control_to_distinct_targets() {
    let text = lower_and_print(
        r#"
      export fn nested(n: u32) -> u32 {
        let outer: u32 = 0;
        let inner: u32 = 0;
        while outer < n {
          while inner < n {
            break;
          }
          continue;
        }
        return outer;
      }
    "#,
    );

    assert!(text.contains("bb5:\n  jump bb6\n"), "{text}");
    assert!(text.contains("bb6:\n  jump bb1\n"), "{text}");
}

#[test]
fn mir_cli_should_match_typescript_oracle_for_official_examples_across_opt_levels() {
    let Some(ts_cli) = typescript_cli() else {
        return;
    };
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
        "bench/perf/fixtures/pricing_helpers.ck",
        "bench/perf/fixtures/pricing_soa.ck",
        "bench/perf/fixtures/f64_kernels.ck",
        "tests/fixtures/f64_edges.ck",
    ];

    for opt_level in 0..=3 {
        let opt_flag = format!("-O{opt_level}");
        for example in examples {
            let source = typescript_root().join(example);

            let ts_output = Command::new("node")
                .arg(&ts_cli)
                .arg("emit-mir")
                .arg(&opt_flag)
                .arg(&source)
                .output()
                .expect("run TypeScript emit-mir");
            assert!(
                ts_output.status.success(),
                "{example} {opt_flag} TS stderr:\n{}",
                String::from_utf8_lossy(&ts_output.stderr)
            );

            let rust_output = Command::new(env!("CARGO_BIN_EXE_ckc"))
                .arg("emit-mir")
                .arg(&opt_flag)
                .arg(&source)
                .output()
                .expect("run Rust emit-mir");
            assert!(
                rust_output.status.success(),
                "{example} {opt_flag} Rust stderr:\n{}",
                String::from_utf8_lossy(&rust_output.stderr)
            );

            assert_eq!(
                String::from_utf8(rust_output.stdout).expect("Rust MIR should be UTF-8"),
                String::from_utf8(ts_output.stdout).expect("TS MIR should be UTF-8"),
                "{example} {opt_flag}"
            );
            assert_eq!(
                String::from_utf8(rust_output.stderr).expect("Rust stderr should be UTF-8"),
                String::from_utf8(ts_output.stderr).expect("TS stderr should be UTF-8"),
                "{example} {opt_flag} stderr"
            );
        }
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
