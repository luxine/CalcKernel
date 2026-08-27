use calckernel::{
    EmitWasmOptions, MirPassBoundsMode, MirPassOverflowMode, MirPassTargetBackend, SourceFile,
    check, emit_wasm_module, emit_wasm_module_with_options, emit_wat_module_with_options,
    lower_to_mir,
};

use super::support::command::node_available;
use super::support::compiler::optimized_module;
use super::support::fixtures;
use super::support::oracle::{typescript_cli, typescript_root};
use std::{
    fs,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

fn emit_wat(source_text: &str, opt_level: u8) -> String {
    let optimized = optimized_module(
        source_text,
        opt_level,
        MirPassOverflowMode::Unchecked,
        MirPassBoundsMode::Unchecked,
        MirPassTargetBackend::Wasm,
    );
    emit_wat_module_with_options(&optimized, EmitWasmOptions { opt_level })
}

fn emit_wasm(source_text: &str, opt_level: u8) -> Vec<u8> {
    let optimized = optimized_module(
        source_text,
        opt_level,
        MirPassOverflowMode::Unchecked,
        MirPassBoundsMode::Unchecked,
        MirPassTargetBackend::Wasm,
    );
    emit_wasm_module_with_options(&optimized, EmitWasmOptions { opt_level })
        .expect("WAT should compile to WASM")
}

#[test]
fn wat_backend_should_emit_scalar_memory_cast_and_dispatcher_text() {
    let wat = emit_wat(
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
        1,
    );

    assert!(wat.contains("(memory (export \"memory\") 1)"));
    assert!(wat.contains("(global (export \"__ck_heap_base\") i32 (i32.const 0))"));
    assert!(wat.contains("(func $add_i64 (export \"add_i64\")"));
    assert!(wat.contains("i64.add"));
    assert!(wat.contains("(local $ik_bb i32)"));
    assert!(wat.contains("loop $ik_dispatch"));
    assert!(wat.contains("i64.load offset=0 align=8"));
    assert!(wat.contains("i64.store offset=0 align=8"));
    assert!(wat.contains("f64.convert_i32_s"));
    assert!(wat.contains("f64.convert_i32_u"));
}

#[test]
fn wasm_backend_should_compile_wat_to_wasm_bytes() {
    let checked = check(&SourceFile::new(
        "test.ck",
        "export fn add_i64(a: i64, b: i64) -> i64 { return a + b; }",
    ));
    assert_eq!(checked.diagnostics, []);
    let mir = lower_to_mir(&checked.checked_program).expect("MIR lowering should succeed");

    let bytes = emit_wasm_module(&mir).expect("WAT should compile to WASM");

    assert_eq!(&bytes[..4], b"\0asm");
    assert_eq!(&bytes[4..8], &[1, 0, 0, 0]);
}

#[test]
fn wasm_backend_should_reject_reachable_print_before_binary_emission() {
    let checked = check(&SourceFile::new(
        "print.ck",
        "export fn api() -> void { print_newline(); }",
    ));
    assert_eq!(checked.diagnostics, []);
    let mir = lower_to_mir(&checked.checked_program).expect("lower print");
    let error = emit_wasm_module(&mir).expect_err("WASM cannot link native print runtime");
    assert!(error.contains("WebAssembly artifact"), "{error}");
    assert!(error.contains("print_newline"), "{error}");
}

#[test]
fn wat_backend_should_emit_structured_while_at_o3_without_dispatcher() {
    let wat = emit_wat(
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
    "#,
        3,
    );

    assert!(wat.contains("block $ik_exit"));
    assert!(wat.contains("loop $ik_loop"));
    assert!(wat.contains("br_if $ik_exit"));
    assert!(wat.contains("br $ik_loop"));
    assert!(!wat.contains("(local $ik_bb i32)"));
    assert!(!wat.contains("loop $ik_dispatch"));
    assert!(!wat.contains("br_table"));
}

#[test]
fn wat_backend_should_match_typescript_oracle_for_official_examples() {
    let Some(ts_cli) = typescript_cli() else {
        return;
    };
    let examples = fixtures::ORACLE_EXAMPLES
        .iter()
        .chain(fixtures::BENCHMARK_FIXTURES)
        .map(|fixture| fixture.oracle)
        .chain(std::iter::once("tests/fixtures/f64_edges.ck"))
        .collect::<Vec<_>>();

    for example in examples {
        let source_path = typescript_root().join(example);
        let source_text = std::fs::read_to_string(&source_path).expect("read TS WAT example");
        let rust_wat = emit_wat(&source_text, 0);
        let ts_output = Command::new("node")
            .arg(&ts_cli)
            .arg("emit-wat")
            .arg(&source_path)
            .output()
            .expect("run TypeScript ckc emit-wat");

        assert!(
            ts_output.status.success(),
            "{example} stderr:\n{}",
            String::from_utf8_lossy(&ts_output.stderr)
        );
        assert_eq!(
            rust_wat,
            String::from_utf8(ts_output.stdout).expect("TS WAT should be UTF-8"),
            "{example}"
        );
    }
}

#[test]
fn wasm_cli_should_match_typescript_oracle_for_official_example_bytes() {
    let Some(ts_cli) = typescript_cli() else {
        return;
    };
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("rust_calckernel_wasm_oracle_{unique}"));
    fs::create_dir_all(&dir).expect("create temp dir");
    let examples = fixtures::ORACLE_EXAMPLES
        .iter()
        .chain(fixtures::BENCHMARK_FIXTURES)
        .map(|fixture| fixture.oracle)
        .chain(std::iter::once("tests/fixtures/f64_edges.ck"))
        .collect::<Vec<_>>();

    for (index, example) in examples.iter().enumerate() {
        let source_path = typescript_root().join(example);
        let wasm_path = dir.join(format!("example_{index}.wasm"));

        let ts_output = Command::new("node")
            .arg(&ts_cli)
            .arg("emit-wasm")
            .arg("--out")
            .arg(&wasm_path)
            .arg(&source_path)
            .output()
            .expect("run TypeScript ckc emit-wasm");
        assert!(
            ts_output.status.success(),
            "{example} TS stderr:\n{}",
            String::from_utf8_lossy(&ts_output.stderr)
        );
        let ts_bytes = fs::read(&wasm_path).expect("read TS wasm");

        let rust_output = Command::new(env!("CARGO_BIN_EXE_ckc"))
            .arg("emit-wasm")
            .arg("--out")
            .arg(&wasm_path)
            .arg(&source_path)
            .output()
            .expect("run Rust ckc emit-wasm");
        assert!(
            rust_output.status.success(),
            "{example} Rust stderr:\n{}",
            String::from_utf8_lossy(&rust_output.stderr)
        );
        let rust_bytes = fs::read(&wasm_path).expect("read Rust wasm");

        assert_eq!(
            String::from_utf8(rust_output.stdout).expect("Rust stdout should be UTF-8"),
            String::from_utf8(ts_output.stdout).expect("TS stdout should be UTF-8"),
            "{example} stdout"
        );
        assert_eq!(rust_bytes, ts_bytes, "{example} wasm bytes");
    }
}

#[test]
fn wasm_cli_should_match_typescript_oracle_for_official_interop_runtime_behavior() {
    let Some(ts_cli) = typescript_cli() else {
        return;
    };
    if !node_available() {
        return;
    }
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("rust_calckernel_wasm_runtime_{unique}"));
    fs::create_dir_all(&dir).expect("create temp dir");
    let runner = dir.join("run_wasm_case.mjs");
    fs::write(&runner, wasm_runtime_runner()).expect("write WASM runtime runner");
    let cases = [
        ("wasm-scalar", fixtures::WASM_SCALAR.oracle),
        ("wasm-calls", fixtures::WASM_CALLS.oracle),
        ("wasm-control-flow", fixtures::WASM_CONTROL_FLOW.oracle),
        ("wasm-memory", fixtures::WASM_MEMORY.oracle),
        ("wasm-short-circuit", fixtures::WASM_SHORT_CIRCUIT.oracle),
        ("dijkstra", fixtures::APPLICATION_DIJKSTRA.oracle),
        ("pricing-aos", fixtures::APPLICATION_PRICING.oracle),
        ("f64-array", fixtures::WASM_F64_ARRAY.oracle),
        ("f64-axpy", fixtures::WASM_F64_AXPY.oracle),
        ("f64-sum", fixtures::WASM_F64_SUM.oracle),
        ("pricing-soa", fixtures::WASM_PRICING_SOA.oracle),
    ];

    for (case_name, example) in cases {
        let source_path = typescript_root().join(example);
        let ts_wasm = dir.join(format!("{case_name}.ts.wasm"));
        let rust_wasm = dir.join(format!("{case_name}.rust.wasm"));

        let ts_emit = Command::new("node")
            .arg(&ts_cli)
            .arg("emit-wasm")
            .arg("-O3")
            .arg("--out")
            .arg(&ts_wasm)
            .arg(&source_path)
            .output()
            .expect("run TypeScript ckc emit-wasm");
        assert!(
            ts_emit.status.success(),
            "{case_name} TS emit stderr:\n{}",
            String::from_utf8_lossy(&ts_emit.stderr)
        );

        let rust_emit = Command::new(env!("CARGO_BIN_EXE_ckc"))
            .arg("emit-wasm")
            .arg("-O3")
            .arg("--out")
            .arg(&rust_wasm)
            .arg(&source_path)
            .output()
            .expect("run Rust ckc emit-wasm");
        assert!(
            rust_emit.status.success(),
            "{case_name} Rust emit stderr:\n{}",
            String::from_utf8_lossy(&rust_emit.stderr)
        );

        let ts_run = Command::new("node")
            .arg(&runner)
            .arg(case_name)
            .arg(&ts_wasm)
            .output()
            .expect("run TS WASM runtime case");
        assert!(
            ts_run.status.success(),
            "{case_name} TS runtime stderr:\n{}",
            String::from_utf8_lossy(&ts_run.stderr)
        );

        let rust_run = Command::new("node")
            .arg(&runner)
            .arg(case_name)
            .arg(&rust_wasm)
            .output()
            .expect("run Rust WASM runtime case");
        assert!(
            rust_run.status.success(),
            "{case_name} Rust runtime stderr:\n{}",
            String::from_utf8_lossy(&rust_run.stderr)
        );

        assert_eq!(
            String::from_utf8(rust_run.stdout).expect("Rust runtime stdout should be UTF-8"),
            String::from_utf8(ts_run.stdout).expect("TS runtime stdout should be UTF-8"),
            "{case_name} runtime stdout"
        );
        assert_eq!(
            String::from_utf8(rust_run.stderr).expect("Rust runtime stderr should be UTF-8"),
            String::from_utf8(ts_run.stderr).expect("TS runtime stderr should be UTF-8"),
            "{case_name} runtime stderr"
        );
    }
}

#[test]
fn wasm_cli_should_match_typescript_oracle_for_f64_edge_fixture_runtime_behavior() {
    let Some(ts_cli) = typescript_cli() else {
        return;
    };
    if !node_available() {
        return;
    }
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("rust_calckernel_wasm_f64_edges_{unique}"));
    fs::create_dir_all(&dir).expect("create temp dir");
    let runner = dir.join("run_wasm_case.mjs");
    fs::write(&runner, wasm_runtime_runner()).expect("write WASM runtime runner");
    let source_path = typescript_root().join("tests/fixtures/f64_edges.ck");
    let ts_wasm = dir.join("f64_edges.ts.wasm");
    let rust_wasm = dir.join("f64_edges.rust.wasm");

    let ts_emit = Command::new("node")
        .arg(&ts_cli)
        .arg("emit-wasm")
        .arg("-O3")
        .arg("--out")
        .arg(&ts_wasm)
        .arg(&source_path)
        .output()
        .expect("run TypeScript ckc emit-wasm");
    assert!(
        ts_emit.status.success(),
        "wasm-f64-edges TS emit stderr:\n{}",
        String::from_utf8_lossy(&ts_emit.stderr)
    );

    let rust_emit = Command::new(env!("CARGO_BIN_EXE_ckc"))
        .arg("emit-wasm")
        .arg("-O3")
        .arg("--out")
        .arg(&rust_wasm)
        .arg(&source_path)
        .output()
        .expect("run Rust ckc emit-wasm");
    assert!(
        rust_emit.status.success(),
        "wasm-f64-edges Rust emit stderr:\n{}",
        String::from_utf8_lossy(&rust_emit.stderr)
    );

    let ts_run = Command::new("node")
        .arg(&runner)
        .arg("wasm-f64-edges")
        .arg(&ts_wasm)
        .output()
        .expect("run TS WASM f64 edge runtime");
    assert!(
        ts_run.status.success(),
        "wasm-f64-edges TS runtime stderr:\n{}",
        String::from_utf8_lossy(&ts_run.stderr)
    );

    let rust_run = Command::new("node")
        .arg(&runner)
        .arg("wasm-f64-edges")
        .arg(&rust_wasm)
        .output()
        .expect("run Rust WASM f64 edge runtime");
    assert!(
        rust_run.status.success(),
        "wasm-f64-edges Rust runtime stderr:\n{}",
        String::from_utf8_lossy(&rust_run.stderr)
    );

    assert_eq!(
        String::from_utf8(rust_run.stdout).expect("Rust runtime stdout should be UTF-8"),
        String::from_utf8(ts_run.stdout).expect("TS runtime stdout should be UTF-8"),
        "wasm-f64-edges runtime stdout"
    );
    assert_eq!(
        String::from_utf8(rust_run.stderr).expect("Rust runtime stderr should be UTF-8"),
        String::from_utf8(ts_run.stderr).expect("TS runtime stderr should be UTF-8"),
        "wasm-f64-edges runtime stderr"
    );
}

#[test]
fn wasm_cli_should_match_typescript_oracle_for_perf_fixture_runtime_behavior() {
    let Some(ts_cli) = typescript_cli() else {
        return;
    };
    if !node_available() {
        return;
    }
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("rust_calckernel_perf_wasm_runtime_{unique}"));
    fs::create_dir_all(&dir).expect("create temp dir");
    let runner = dir.join("run_wasm_case.mjs");
    fs::write(&runner, wasm_runtime_runner()).expect("write WASM runtime runner");
    let cases = [
        (
            "bench-pricing-helpers",
            fixtures::BENCH_PRICING_HELPERS.oracle,
        ),
        ("bench-pricing-soa", fixtures::BENCH_PRICING_SOA.oracle),
        ("bench-f64-kernels", fixtures::BENCH_F64_KERNELS.oracle),
    ];

    for (case_name, fixture) in cases {
        let source_path = typescript_root().join(fixture);
        let ts_wasm = dir.join(format!("{case_name}.ts.wasm"));
        let rust_wasm = dir.join(format!("{case_name}.rust.wasm"));

        let ts_emit = Command::new("node")
            .arg(&ts_cli)
            .arg("emit-wasm")
            .arg("-O3")
            .arg("--out")
            .arg(&ts_wasm)
            .arg(&source_path)
            .output()
            .expect("run TypeScript ckc emit-wasm");
        assert!(
            ts_emit.status.success(),
            "{case_name} TS emit stderr:\n{}",
            String::from_utf8_lossy(&ts_emit.stderr)
        );

        let rust_emit = Command::new(env!("CARGO_BIN_EXE_ckc"))
            .arg("emit-wasm")
            .arg("-O3")
            .arg("--out")
            .arg(&rust_wasm)
            .arg(&source_path)
            .output()
            .expect("run Rust ckc emit-wasm");
        assert!(
            rust_emit.status.success(),
            "{case_name} Rust emit stderr:\n{}",
            String::from_utf8_lossy(&rust_emit.stderr)
        );

        let ts_run = Command::new("node")
            .arg(&runner)
            .arg(case_name)
            .arg(&ts_wasm)
            .output()
            .expect("run TS WASM runtime case");
        assert!(
            ts_run.status.success(),
            "{case_name} TS runtime stderr:\n{}",
            String::from_utf8_lossy(&ts_run.stderr)
        );

        let rust_run = Command::new("node")
            .arg(&runner)
            .arg(case_name)
            .arg(&rust_wasm)
            .output()
            .expect("run Rust WASM runtime case");
        assert!(
            rust_run.status.success(),
            "{case_name} Rust runtime stderr:\n{}",
            String::from_utf8_lossy(&rust_run.stderr)
        );

        assert_eq!(
            String::from_utf8(rust_run.stdout).expect("Rust runtime stdout should be UTF-8"),
            String::from_utf8(ts_run.stdout).expect("TS runtime stdout should be UTF-8"),
            "{case_name} runtime stdout"
        );
        assert_eq!(
            String::from_utf8(rust_run.stderr).expect("Rust runtime stderr should be UTF-8"),
            String::from_utf8(ts_run.stderr).expect("TS runtime stderr should be UTF-8"),
            "{case_name} runtime stderr"
        );
    }
}

#[test]
fn wasm_backend_should_run_break_continue_dispatcher_fallback_at_all_opt_levels() {
    if !node_available() {
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
    let wat = emit_wat(source, 3);
    assert!(wat.contains("loop $ik_dispatch"), "{wat}");

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("rust_calckernel_break_continue_wasm_{unique}"));
    fs::create_dir_all(&dir).expect("create temp dir");
    let runner = r#"
const fs = require("node:fs");
WebAssembly.instantiate(fs.readFileSync(process.argv[1]))
  .then(({ instance }) => {
    const { early_exit, skip_three, nested, return_from_loop } = instance.exports;
    if (early_exit(10) !== 3 || skip_three(5) !== 12 || nested(4) !== 8 ||
        return_from_loop(5) !== 99) {
      process.exit(1);
    }
  })
  .catch((error) => {
    console.error(error);
    process.exit(1);
  });
"#;

    for opt_level in 0..=3 {
        let wasm_path = dir.join(format!("control_o{opt_level}.wasm"));
        fs::write(&wasm_path, emit_wasm(source, opt_level)).expect("write WASM");
        let run = Command::new("node")
            .arg("-e")
            .arg(runner)
            .arg(&wasm_path)
            .output()
            .expect("run WASM harness");
        assert!(
            run.status.success(),
            "O{opt_level} runtime stderr:\n{}",
            String::from_utf8_lossy(&run.stderr)
        );
    }
}

#[test]
fn wasm_backend_should_emit_and_run_void_functions_without_results() {
    if !node_available() {
        return;
    }
    let source = r#"
      fn set_one(out: ptr<i32>) -> void { out[0] = 1; }

      export fn call_path(out: ptr<i32>) -> void { set_one(out); }

      export fn conditional(out: ptr<i32>, write: bool) -> void {
        if !write { return; }
        out[0] = 7;
      }

      export fn loop_path(out: ptr<i32>) -> void {
        let i: u32 = 0;
        while i < 1 {
          out[i] = 42;
          i = i + 1;
        }
      }
    "#;
    let wat = emit_wat(source, 3);
    assert!(
        wat.contains("(func $set_one\n    (param $out i32)"),
        "{wat}"
    );
    assert!(wat.contains("call $set_one"), "{wat}");
    assert!(!wat.contains("(result"), "{wat}");
    assert!(wat.contains("loop $ik_loop"), "{wat}");

    let runner = r#"
const fs = require("node:fs");
WebAssembly.instantiate(fs.readFileSync(process.argv[1]))
  .then(({ instance }) => {
    const view = new DataView(instance.exports.memory.buffer);
    view.setInt32(0, 0, true);
    if (instance.exports.call_path(0) !== undefined || view.getInt32(0, true) !== 1) process.exit(1);
    instance.exports.conditional(0, 0);
    if (view.getInt32(0, true) !== 1) process.exit(2);
    instance.exports.conditional(0, 1);
    if (view.getInt32(0, true) !== 7) process.exit(3);
    instance.exports.loop_path(0);
    if (view.getInt32(0, true) !== 42) process.exit(4);
  })
  .catch((error) => { console.error(error); process.exit(5); });
"#;
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("rust_calckernel_void_wasm_{unique}"));
    fs::create_dir_all(&dir).expect("create temp dir");
    for opt_level in 0..=3 {
        let wasm = dir.join(format!("void_o{opt_level}.wasm"));
        fs::write(&wasm, emit_wasm(source, opt_level)).expect("write wasm");
        let output = Command::new("node")
            .arg("-e")
            .arg(runner)
            .arg(&wasm)
            .output()
            .expect("run node");
        assert!(
            output.status.success(),
            "O{opt_level} node stderr:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn wasm_backend_should_flatten_slice_params_collision_safely() {
    let wat = emit_wat(
        r#"
      export fn collide(
        items: slice<i32>,
        items_data: ptr<i32>,
        items_len: u32
      ) -> i32 {
        return items[0] + items_data[items_len];
      }
    "#,
        0,
    );

    assert!(wat.contains("(param $items_data_1 i32)"), "{wat}");
    assert!(wat.contains("(param $items_len_1 i32)"), "{wat}");
    assert!(wat.contains("(param $items_data i32)"), "{wat}");
    assert!(wat.contains("(param $items_len i32)"), "{wat}");
}

#[test]
fn wasm_backend_should_emit_paired_slice_locals_temps_moves_and_projections() {
    let wat = emit_wat(
        r#"
      export fn view(data: ptr<i32>, len: u32, start: u32, end: u32) -> u32 {
        let items: slice<i32> = slice(data, len);
        let copy: slice<i32> = items;
        let middle: slice<i32> = copy[start..end];
        return middle.len;
      }
    "#,
        1,
    );

    for local in [
        "(local $items_data i32)",
        "(local $items_len i32)",
        "(local $copy_data i32)",
        "(local $copy_len i32)",
        "(local $middle_data i32)",
        "(local $middle_len i32)",
    ] {
        assert!(wat.contains(local), "missing {local}:\n{wat}");
    }
    assert!(wat.contains("local.get $items_data"), "{wat}");
    assert!(wat.contains("local.set $copy_data"), "{wat}");
    assert!(wat.contains("local.get $middle_len"), "{wat}");
}

#[test]
fn wasm_backend_should_load_store_slice_fields_with_size8_align4() {
    let wat = emit_wat(
        r#"
      struct Bundle {
        tag: i32;
        items: slice<i32>;
        tail: i32;
      }

      export fn round_trip(bundle: ptr<Bundle>, items: slice<i32>) -> u32 {
        bundle[0].items = items;
        let copy: slice<i32> = bundle[0].items;
        return copy.len;
      }
    "#,
        1,
    );

    assert!(wat.contains("i32.store offset=0 align=4"), "{wat}");
    assert!(wat.contains("i32.store offset=4 align=4"), "{wat}");
    assert!(wat.contains("i32.load offset=0 align=4"), "{wat}");
    assert!(wat.contains("i32.load offset=4 align=4"), "{wat}");
    assert!(
        wat.contains("i32.const 4\n"),
        "slice field offset missing:\n{wat}"
    );
}

#[test]
fn wasm_backend_should_return_internal_slices_as_two_values() {
    let source = r#"
      fn direct(items: slice<i32>) -> slice<i32> {
        return items[0..1];
      }

      fn choose(items: slice<i32>, second: bool) -> slice<i32> {
        if second {
          return items[1..2];
        }
        return direct(items);
      }

      fn drop_one(items: slice<i32>) -> slice<i32> {
        let i: u32 = 0;
        while i < 1 {
          i = i + 1;
        }
        return items[i..items.len];
      }

      export fn selected(items: slice<i32>, second: bool) -> i32 {
        let result: slice<i32> = choose(items, second);
        return result[0];
      }

      export fn dropped(items: slice<i32>) -> i32 {
        let result: slice<i32> = drop_one(items);
        return result[0];
      }
    "#;
    let wat = emit_wat(source, 3);

    assert!(wat.contains("(func $direct"), "{wat}");
    assert!(wat.matches("(result i32 i32)").count() >= 3, "{wat}");
    assert!(wat.contains("(local $ik_ret_data i32)"), "{wat}");
    assert!(wat.contains("(local $ik_ret_len i32)"), "{wat}");
    let call = wat.find("call $choose").expect("slice-returning call");
    let after_call = &wat[call..];
    let set_len = after_call.find("local.set $t").expect("first result pop");
    let set_data = after_call[set_len + 1..]
        .find("local.set $t")
        .expect("second result pop")
        + set_len
        + 1;
    assert!(set_len < set_data, "{after_call}");
    assert!(
        wat.contains("loop $ik_loop"),
        "structured return path missing:\n{wat}"
    );
}

#[test]
fn wasm_backend_should_run_slice_index_subslice_and_struct_elements_at_all_levels() {
    if !node_available() {
        return;
    }
    let source = r#"
      struct Pair {
        left: i32;
        right: i32;
      }

      fn choose(items: slice<Pair>, second: bool) -> slice<Pair> {
        if second {
          return items[1..2];
        }
        return items[0..1];
      }

      export fn sum_selected(items: slice<Pair>, second: bool) -> i32 {
        let selected: slice<Pair> = choose(items, second);
        return selected[0].left + selected[0].right;
      }
    "#;
    let runner = r#"
const fs = require("node:fs");
WebAssembly.instantiate(fs.readFileSync(process.argv[1]))
  .then(({ instance }) => {
    const view = new DataView(instance.exports.memory.buffer);
    view.setInt32(0, 2, true);
    view.setInt32(4, 3, true);
    view.setInt32(8, 7, true);
    view.setInt32(12, 11, true);
    if (instance.exports.sum_selected(0, 2, 0) !== 5) process.exit(1);
    if (instance.exports.sum_selected(0, 2, 1) !== 18) process.exit(2);
  })
  .catch((error) => { console.error(error); process.exit(3); });
"#;
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("rust_calckernel_slice_wasm_{unique}"));
    fs::create_dir_all(&dir).expect("create temp dir");
    for opt_level in 0..=3 {
        let wasm = dir.join(format!("slice_o{opt_level}.wasm"));
        fs::write(&wasm, emit_wasm(source, opt_level)).expect("write wasm");
        let output = Command::new("node")
            .arg("-e")
            .arg(runner)
            .arg(&wasm)
            .output()
            .expect("run node");
        assert!(
            output.status.success(),
            "O{opt_level} node stderr:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

fn wasm_runtime_runner() -> &'static str {
    r#"
import { readFileSync } from "node:fs";

const [caseName, wasmPath] = process.argv.slice(2);
const bytes = readFileSync(wasmPath);
const { instance } = await WebAssembly.instantiate(bytes);
const { memory } = instance.exports;
if (!(memory instanceof WebAssembly.Memory)) {
  throw new Error("generated module did not export memory");
}

function close(actual, expected) {
  return Math.abs(actual - expected) < 0.0000001;
}

function closeF64(actual, expected) {
  const diff = Math.abs(actual - expected);
  const scale = Math.max(Math.abs(actual), Math.abs(expected), 1.0);
  return diff <= 0.000000000001 * scale || diff <= 0.000000000001;
}

function classifyF64(value) {
  if (Number.isNaN(value)) {
    return "nan";
  }
  if (!Number.isFinite(value)) {
    return Object.is(value, -Infinity) ? "-inf" : "+inf";
  }
  if (Object.is(value, -0)) {
    return "-0";
  }
  if (Object.is(value, 0)) {
    return "+0";
  }
  return "finite";
}

function classIs(value, expected) {
  return classifyF64(value) === expected;
}

function ok(value) {
  return value ? "ok" : "fail";
}

function expectedTotal(price, quantity, discount, taxRatePpm) {
  const subtotal = price * quantity;
  const afterDiscount = subtotal - discount;
  const tax = (afterDiscount * taxRatePpm) / 1000000n;
  return afterDiscount + tax;
}

function runWasmScalar() {
  const addI32 = instance.exports.add_i32;
  const addI64 = instance.exports.add_i64;
  const lessI64 = instance.exports.less_i64;
  const divU64 = instance.exports.div_u64;
  if (
    typeof addI32 !== "function" ||
    typeof addI64 !== "function" ||
    typeof lessI64 !== "function" ||
    typeof divU64 !== "function"
  ) {
    throw new Error("generated scalar WASM did not export the expected functions");
  }
  const values = {
    addI32: addI32(1, 2),
    addI64: addI64(1n, 2n),
    lessI64: lessI64(1n, 2n),
    divU64: divU64(10n, 2n)
  };
  if (values.addI32 !== 3 || values.addI64 !== 3n || values.lessI64 !== 1 || values.divU64 !== 5n) {
    throw new Error(`wasm-scalar mismatch ${JSON.stringify(values, (_, value) => typeof value === "bigint" ? value.toString() : value)}`);
  }
  return `wasm-scalar:add_i32=${values.addI32};add_i64=${values.addI64};less_i64=${values.lessI64};div_u64=${values.divU64}`;
}

function runWasmCalls() {
  const calc = instance.exports.calc;
  if (typeof calc !== "function") {
    throw new Error("generated call WASM did not export calc");
  }
  if (instance.exports.add_i64 !== undefined || instance.exports.double_i64 !== undefined) {
    throw new Error("generated call WASM exported non-export helper functions");
  }
  const result = calc(1n, 2n);
  if (result !== 6n) {
    throw new Error(`wasm-calls mismatch result=${result}`);
  }
  return `wasm-calls:calc=${result};helpers=private`;
}

function runWasmControlFlow() {
  const maxI32 = instance.exports.max_i32;
  const sumToN = instance.exports.sum_to_n;
  if (typeof maxI32 !== "function" || typeof sumToN !== "function") {
    throw new Error("generated control-flow WASM did not export the expected functions");
  }
  const high = maxI32(10, 3);
  const low = maxI32(1, 3);
  const sum = sumToN(5n);
  if (high !== 10 || low !== 3 || sum !== 10n) {
    throw new Error(`wasm-control-flow mismatch high=${high} low=${low} sum=${sum}`);
  }
  return `wasm-control-flow:max=${high},${low};sum=${sum}`;
}

function writeItem(view, offset, fields) {
  view.setBigInt64(offset + 0, fields.price, true);
  view.setBigInt64(offset + 8, fields.qty, true);
  view.setBigInt64(offset + 16, fields.discount, true);
  view.setBigInt64(offset + 24, fields.taxRatePpm, true);
}

function runWasmMemory() {
  const firstPrice = instance.exports.first_price;
  const getPrice = instance.exports.get_price;
  const writeI64 = instance.exports.write_i64;
  if (typeof firstPrice !== "function" || typeof getPrice !== "function" || typeof writeI64 !== "function") {
    throw new Error("generated memory WASM did not export the expected functions");
  }
  const view = new DataView(memory.buffer);
  writeItem(view, 0, { price: 1234n, qty: 2n, discount: 3n, taxRatePpm: 4n });
  const first = firstPrice(0);
  const base = 128;
  const itemSize = 32;
  writeItem(view, base, { price: 11n, qty: 0n, discount: 0n, taxRatePpm: 0n });
  writeItem(view, base + itemSize, { price: 222n, qty: 0n, discount: 0n, taxRatePpm: 0n });
  const indexed = getPrice(base, 1);
  const outOffset = 512;
  const status = writeI64(outOffset, 123n);
  const stored = view.getBigInt64(outOffset, true);
  if (first !== 1234n || indexed !== 222n || status !== 0 || stored !== 123n) {
    throw new Error(`wasm-memory mismatch first=${first} indexed=${indexed} status=${status} stored=${stored}`);
  }
  return `wasm-memory:first=${first};indexed=${indexed};status=${status};stored=${stored}`;
}

function runWasmShortCircuit() {
  const andShortCircuit = instance.exports.and_short_circuit;
  const orShortCircuit = instance.exports.or_short_circuit;
  if (typeof andShortCircuit !== "function" || typeof orShortCircuit !== "function") {
    throw new Error("generated short-circuit WASM did not export the expected functions");
  }
  const values = [
    andShortCircuit(0n, 10n),
    andShortCircuit(2n, 10n),
    orShortCircuit(0n, 10n),
    orShortCircuit(2n, 10n)
  ];
  const expected = [0, 1, 1, 1];
  if (values.some((value, index) => value !== expected[index])) {
    throw new Error(`wasm-short-circuit mismatch actual=${values.join(",")} expected=${expected.join(",")}`);
  }
  return `wasm-short-circuit:out=${values.join(",")}`;
}

function runDijkstra() {
  const dijkstraMatrix = instance.exports.dijkstra_matrix;
  if (typeof dijkstraMatrix !== "function") {
    throw new Error("generated dijkstra WASM did not export dijkstra_matrix");
  }
  const view = new DataView(memory.buffer);
  const nodeCount = 5;
  const inf = 1000000n;
  const configOffset = 0;
  const weightsOffset = 64;
  const distOffset = 512;
  const prevOffset = 768;
  const visitedOffset = 896;
  const weights = [
    0n, 2n, 5n, 0n, 0n,
    0n, 0n, 1n, 2n, 9n,
    0n, 0n, 0n, 1n, 0n,
    0n, 0n, 0n, 0n, 3n,
    0n, 0n, 0n, 0n, 0n
  ];

  view.setInt32(configOffset + 0, nodeCount, true);
  view.setInt32(configOffset + 4, 0, true);
  view.setBigInt64(configOffset + 8, inf, true);
  for (const [index, weight] of weights.entries()) {
    view.setBigInt64(weightsOffset + index * 8, weight, true);
  }

  const settled = dijkstraMatrix(configOffset, weightsOffset, distOffset, prevOffset, visitedOffset);
  const actualDist = Array.from({ length: nodeCount }, (_, index) => view.getBigInt64(distOffset + index * 8, true));
  const actualPrev = Array.from({ length: nodeCount }, (_, index) => view.getInt32(prevOffset + index * 4, true));
  const actualVisited = Array.from({ length: nodeCount }, (_, index) => view.getInt32(visitedOffset + index * 4, true));
  const expectedDist = [0n, 2n, 3n, 4n, 7n];
  const expectedPrev = [0, 0, 1, 1, 3];
  const expectedVisited = [1, 1, 1, 1, 1];
  if (
    settled !== nodeCount ||
    actualDist.some((value, index) => value !== expectedDist[index]) ||
    actualPrev.some((value, index) => value !== expectedPrev[index]) ||
    actualVisited.some((value, index) => value !== expectedVisited[index])
  ) {
    throw new Error(
      `dijkstra mismatch settled=${settled} dist=${actualDist.join(",")} ` +
        `prev=${actualPrev.join(",")} visited=${actualVisited.join(",")}`
    );
  }
  return (
    `dijkstra:settled=${settled};` +
    `dist=${actualDist.join(",")};` +
    `prev=${actualPrev.join(",")};` +
    `visited=${actualVisited.join(",")}`
  );
}

function runPricingAos() {
  const calcItems = instance.exports.calc_items;
  if (typeof calcItems !== "function") {
    throw new Error("generated WASM did not export calc_items");
  }
  const view = new DataView(memory.buffer);
  const itemsOffset = 0;
  const outOffset = 4096;
  const itemSize = 32;
  const items = [
    { price: 10000n, qty: 2n, discount: 1000n, taxRatePpm: 82500n },
    { price: 2500n, qty: 4n, discount: 0n, taxRatePpm: 100000n },
    { price: 1200n, qty: 5n, discount: 500n, taxRatePpm: 100000n }
  ];
  for (let index = 0; index < items.length; index += 1) {
    const base = itemsOffset + index * itemSize;
    const item = items[index];
    view.setBigInt64(base + 0, item.price, true);
    view.setBigInt64(base + 8, item.qty, true);
    view.setBigInt64(base + 16, item.discount, true);
    view.setBigInt64(base + 24, item.taxRatePpm, true);
  }
  const status = calcItems(itemsOffset, items.length, outOffset);
  const actual = items.map((_, index) => view.getBigInt64(outOffset + index * 8, true));
  const expected = items.map((item) => expectedTotal(item.price, item.qty, item.discount, item.taxRatePpm));
  if (status !== 0 || actual.some((value, index) => value !== expected[index])) {
    throw new Error(`pricing-aos mismatch status=${status} actual=${actual.join(",")} expected=${expected.join(",")}`);
  }
  return `pricing-aos:status=${status};out=${actual.join(",")}`;
}

function runF64Array() {
  const axpyF64 = instance.exports.axpy_f64;
  if (typeof axpyF64 !== "function") {
    throw new Error("generated WASM did not export axpy_f64");
  }
  const values = new Float64Array(memory.buffer);
  const len = 4;
  const factor = 1.25;
  const xOffset = 0;
  const yOffset = 8 * 8;
  const xIndex = xOffset / 8;
  const yIndex = yOffset / 8;
  values.set([1.0, 2.0, 3.0, 4.0], xIndex);
  values.set([0.5, 1.25, 1.25, 2.0], yIndex);
  const checksum = axpyF64(factor, xOffset, yOffset, len);
  const actual = Array.from(values.subarray(yIndex, yIndex + len));
  const expected = [1.75, 3.75, 5.0, 7.0];
  const expectedChecksum = expected.reduce((sum, value) => sum + value, 0.0);
  if (!close(checksum, expectedChecksum) || actual.some((value, index) => !close(value, expected[index]))) {
    throw new Error(`f64-array mismatch checksum=${checksum} actual=${actual.join(",")}`);
  }
  return `f64-array:checksum=${checksum};out=${actual.join(",")}`;
}

function runF64Axpy() {
  const axpyF64 = instance.exports.axpy_f64;
  if (typeof axpyF64 !== "function") {
    throw new Error("generated WASM did not export axpy_f64");
  }
  const values = new Float64Array(memory.buffer);
  const xInput = [1.0, 2.0, 3.0, 4.0];
  const yInput = [0.5, -1.0, 10.0, 20.0];
  const xIndex = 0;
  const yIndex = 8;
  values.set(xInput, xIndex);
  values.set(yInput, yIndex);
  const checksum = axpyF64(2.0, xIndex * 8, yIndex * 8, xInput.length);
  const actual = Array.from(values.subarray(yIndex, yIndex + xInput.length));
  const expected = xInput.map((value, index) => 2.0 * value + yInput[index]);
  const expectedChecksum = expected.reduce((sum, value) => sum + value, 0.0);
  if (!close(checksum, expectedChecksum) || actual.some((value, index) => !close(value, expected[index]))) {
    throw new Error(`f64-axpy mismatch checksum=${checksum} actual=${actual.join(",")}`);
  }
  return `f64-axpy:checksum=${checksum};out=${actual.join(",")}`;
}

function runF64Sum() {
  const sumF64 = instance.exports.sum_f64;
  if (typeof sumF64 !== "function") {
    throw new Error("generated WASM did not export sum_f64");
  }
  const input = [1.25, -2.5, 3.75, 4.5, 10.0];
  new Float64Array(memory.buffer).set(input, 0);
  const actual = sumF64(0, input.length);
  const expected = input.reduce((sum, value) => sum + value, 0.0);
  if (!close(actual, expected)) {
    throw new Error(`f64-sum mismatch actual=${actual} expected=${expected}`);
  }
  return `f64-sum:result=${actual};inputLength=${input.length}`;
}

function runF64Edges() {
  const f64Exports = [
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
    "underflow_smoke"
  ];
  const boolExports = [
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
    "negative_infinity_less_than_finite"
  ];
  const pointerExports = [
    "ptr_read",
    "ptr_write",
    "struct_read",
    "struct_write",
    "nested_struct_read",
    "nested_struct_write"
  ];
  for (const name of [...f64Exports, ...boolExports, ...pointerExports]) {
    if (typeof instance.exports[name] !== "function") {
      throw new Error(`generated f64 edge WASM did not export ${name}`);
    }
  }

  const view = new DataView(memory.buffer);
  const valuesOffset = 0;
  const quotesOffset = 64;
  const nestedOffset = 128;
  const quoteSize = 16;
  const nestedSize = 24;

  for (const [index, value] of [1.0, 2.5, 4.0].entries()) {
    view.setFloat64(valuesOffset + index * 8, value, true);
  }
  const quotes = [
    { price: 10.25, tax: 0.75 },
    { price: 20.5, tax: 1.25 }
  ];
  for (const [index, quote] of quotes.entries()) {
    const base = quotesOffset + index * quoteSize;
    view.setFloat64(base + 0, quote.price, true);
    view.setFloat64(base + 8, quote.tax, true);
  }
  const nested = [
    { price: 1.25, tax: 0.75, fee: 2.0 },
    { price: 10.0, tax: 2.0, fee: 3.0 }
  ];
  for (const [index, item] of nested.entries()) {
    const base = nestedOffset + index * nestedSize;
    view.setFloat64(base + 0, item.price, true);
    view.setFloat64(base + 8, item.tax, true);
    view.setFloat64(base + 16, item.fee, true);
  }

  const ptrStoreValue = instance.exports.ptr_write(valuesOffset, 1, 8.75);
  const structWriteValue = instance.exports.struct_write(quotesOffset, 1, 0.5);
  const nestedWriteValue = instance.exports.nested_struct_write(nestedOffset, 1, 1.5);

  const checks = [
    ["finite_add", closeF64(instance.exports.finite_add(), 4.0)],
    ["finite_sub", closeF64(instance.exports.finite_sub(), 3.5)],
    ["finite_mul", closeF64(instance.exports.finite_mul(), 3.75)],
    ["finite_div", closeF64(instance.exports.finite_div(), 3.5)],
    ["tolerance_calc", closeF64(instance.exports.tolerance_calc(), 10.0)],
    ["finite_less", instance.exports.finite_less() === 1],
    ["finite_less_equal", instance.exports.finite_less_equal() === 1],
    ["finite_equal", instance.exports.finite_equal() === 1],
    ["pos_inf", classIs(instance.exports.positive_infinity(), "+inf")],
    ["neg_inf", classIs(instance.exports.negative_infinity(), "-inf")],
    ["nan", classIs(instance.exports.not_a_number(), "nan")],
    ["neg_zero", classIs(instance.exports.negative_zero(), "-0")],
    ["zero_eq_neg_zero", instance.exports.zero_equals_negative_zero() === 1],
    ["nan_eq_nan", instance.exports.nan_equals_nan() === 0],
    ["nan_ne_nan", instance.exports.nan_not_equals_nan() === 1],
    ["nan_lt_one", instance.exports.nan_less_than_one() === 0],
    ["nan_le_one", instance.exports.nan_less_equal_one() === 0],
    ["nan_gt_one", instance.exports.nan_greater_than_one() === 0],
    ["nan_ge_one", instance.exports.nan_greater_equal_one() === 0],
    ["inf_plus", classIs(instance.exports.infinity_plus_finite(), "+inf")],
    ["inf_minus_inf", classIs(instance.exports.infinity_minus_infinity(), "nan")],
    ["overflow", classIs(instance.exports.overflow_to_infinity(), "+inf")],
    ["underflow", classIs(instance.exports.underflow_smoke(), "+0")],
    ["inf_gt_finite", instance.exports.infinity_greater_than_finite() === 1],
    ["neg_inf_lt_finite", instance.exports.negative_infinity_less_than_finite() === 1],
    ["ptr_load", closeF64(instance.exports.ptr_read(valuesOffset, 2), 4.0)],
    ["ptr_store", closeF64(ptrStoreValue, 8.75) && closeF64(view.getFloat64(valuesOffset + 8, true), 8.75)],
    ["struct_read", closeF64(instance.exports.struct_read(quotesOffset, 0), 11.0)],
    ["struct_write", closeF64(structWriteValue, 21.0) && closeF64(view.getFloat64(quotesOffset + quoteSize + 8, true), 0.5)],
    ["nested_struct_read", closeF64(instance.exports.nested_struct_read(nestedOffset, 0), 4.0)],
    [
      "nested_struct_write",
      closeF64(nestedWriteValue, 14.5) && closeF64(view.getFloat64(nestedOffset + nestedSize + 8, true), 1.5)
    ]
  ];
  return `wasm-f64-edges:${checks.map(([name, passed]) => `${name}=${ok(passed)}`).join(";")}`;
}

function runPricingSoa() {
  const pricingSoA = instance.exports.pricing_soa;
  if (typeof pricingSoA !== "function") {
    throw new Error("generated WASM did not export pricing_soa");
  }
  const rows = [
    { price: 10000n, quantity: 2n, discount: 1000n, taxRatePpm: 82500n },
    { price: 2500n, quantity: 4n, discount: 0n, taxRatePpm: 100000n },
    { price: 1200n, quantity: 5n, discount: 500n, taxRatePpm: 100000n },
    { price: 999n, quantity: 3n, discount: 100n, taxRatePpm: 62500n }
  ];
  const values = new BigInt64Array(memory.buffer);
  const len = rows.length;
  const pricesIndex = 0;
  const quantitiesIndex = pricesIndex + len;
  const discountsIndex = quantitiesIndex + len;
  const taxRatesIndex = discountsIndex + len;
  const outIndex = taxRatesIndex + len;
  for (let index = 0; index < len; index += 1) {
    values[pricesIndex + index] = rows[index].price;
    values[quantitiesIndex + index] = rows[index].quantity;
    values[discountsIndex + index] = rows[index].discount;
    values[taxRatesIndex + index] = rows[index].taxRatePpm;
  }
  const status = pricingSoA(pricesIndex * 8, quantitiesIndex * 8, discountsIndex * 8, taxRatesIndex * 8, outIndex * 8, len);
  const actual = Array.from(values.subarray(outIndex, outIndex + len));
  const expected = rows.map((row) => expectedTotal(row.price, row.quantity, row.discount, row.taxRatePpm));
  if (status !== 0 || actual.some((value, index) => value !== expected[index])) {
    throw new Error(`pricing-soa mismatch status=${status} actual=${actual.join(",")} expected=${expected.join(",")}`);
  }
  return `pricing-soa:status=${status};out=${actual.join(",")}`;
}

function runBenchPricingHelpers() {
  if (
    instance.exports.item_subtotal !== undefined ||
    instance.exports.apply_discount !== undefined ||
    instance.exports.calc_tax !== undefined
  ) {
    throw new Error("generated benchmark helper WASM exported private helper functions");
  }
  return runPricingAos().replace("pricing-aos:", "bench-pricing-helpers:");
}

function runBenchPricingSoa() {
  return runPricingSoa().replace("pricing-soa:", "bench-pricing-soa:");
}

function runBenchF64Kernels() {
  const axpyF64 = instance.exports.axpy_f64;
  const dotF64 = instance.exports.dot_f64;
  const sumF64 = instance.exports.sum_f64;
  const scaleF64 = instance.exports.scale_f64;
  if (
    typeof axpyF64 !== "function" ||
    typeof dotF64 !== "function" ||
    typeof sumF64 !== "function" ||
    typeof scaleF64 !== "function"
  ) {
    throw new Error("generated WASM did not export the expected f64 kernels");
  }
  const values = new Float64Array(memory.buffer);
  const len = 4;
  const xInput = [1.0, -2.0, 3.5, 4.25];
  const yInput = [0.5, 8.0, -1.5, 2.25];
  const xIndex = 0;
  const yIndex = 16;
  const scaleIndex = 32;
  values.set(xInput, xIndex);
  values.set(yInput, yIndex);
  const axpyChecksum = axpyF64(1.5, xIndex * 8, yIndex * 8, len);
  const axpyActual = Array.from(values.subarray(yIndex, yIndex + len));
  const axpyExpected = xInput.map((value, index) => 1.5 * value + yInput[index]);
  const axpyExpectedChecksum = axpyExpected.reduce((sum, value) => sum + value, 0.0);
  if (
    !close(axpyChecksum, axpyExpectedChecksum) ||
    axpyActual.some((value, index) => !close(value, axpyExpected[index]))
  ) {
    throw new Error(`bench-f64-kernels axpy mismatch checksum=${axpyChecksum} out=${axpyActual.join(",")}`);
  }

  values.set(xInput, xIndex);
  values.set(yInput, yIndex);
  const dotActual = dotF64(xIndex * 8, yIndex * 8, len);
  const dotExpected = xInput.reduce((sum, value, index) => sum + value * yInput[index], 0.0);
  if (!close(dotActual, dotExpected)) {
    throw new Error(`bench-f64-kernels dot mismatch actual=${dotActual} expected=${dotExpected}`);
  }

  const sumActual = sumF64(xIndex * 8, len);
  const sumExpected = xInput.reduce((sum, value) => sum + value, 0.0);
  if (!close(sumActual, sumExpected)) {
    throw new Error(`bench-f64-kernels sum mismatch actual=${sumActual} expected=${sumExpected}`);
  }

  const scaleInput = [0.25, -1.5, 2.0, 10.0];
  values.set(scaleInput, scaleIndex);
  const scaleChecksum = scaleF64(-2.0, scaleIndex * 8, len);
  const scaleActual = Array.from(values.subarray(scaleIndex, scaleIndex + len));
  const scaleExpected = scaleInput.map((value) => -2.0 * value);
  const scaleExpectedChecksum = scaleExpected.reduce((sum, value) => sum + value, 0.0);
  if (
    !close(scaleChecksum, scaleExpectedChecksum) ||
    scaleActual.some((value, index) => !close(value, scaleExpected[index]))
  ) {
    throw new Error(`bench-f64-kernels scale mismatch checksum=${scaleChecksum} out=${scaleActual.join(",")}`);
  }

  return `bench-f64-kernels:axpy=${axpyChecksum};dot=${dotActual};sum=${sumActual};scale=${scaleChecksum};axpyOut=${axpyActual.join(",")};scaleOut=${scaleActual.join(",")}`;
}

const runners = {
  "wasm-scalar": runWasmScalar,
  "wasm-calls": runWasmCalls,
  "wasm-control-flow": runWasmControlFlow,
  "wasm-memory": runWasmMemory,
  "wasm-short-circuit": runWasmShortCircuit,
  "dijkstra": runDijkstra,
  "pricing-aos": runPricingAos,
  "f64-array": runF64Array,
  "f64-axpy": runF64Axpy,
  "f64-sum": runF64Sum,
  "wasm-f64-edges": runF64Edges,
  "pricing-soa": runPricingSoa,
  "bench-pricing-helpers": runBenchPricingHelpers,
  "bench-pricing-soa": runBenchPricingSoa,
  "bench-f64-kernels": runBenchF64Kernels
};

const runner = runners[caseName];
if (!runner) {
  throw new Error(`unknown case: ${caseName}`);
}
console.log(runner());
"#
}
