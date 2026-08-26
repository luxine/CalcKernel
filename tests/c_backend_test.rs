use std::{
    fs,
    path::PathBuf,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

use calckernel::{
    BoundsMode, EmitCOptions, MirPassBoundsMode, MirPassContext, MirPassOverflowMode,
    MirPassTargetBackend, OverflowMode, SourceFile, build_mir_optimization_pipeline, check,
    emit_c_header, emit_c_module, emit_c_module_with_header, lower_to_mir, run_mir_pass_pipeline,
};

#[path = "support/fixtures.rs"]
mod fixtures;

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
    emit_c_with_modes_and_opt_level(source_text, overflow_mode, BoundsMode::Unchecked, opt_level)
}

fn emit_c_with_modes_and_opt_level(
    source_text: &str,
    overflow_mode: OverflowMode,
    bounds_mode: BoundsMode,
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
            bounds_mode: match bounds_mode {
                BoundsMode::Unchecked => MirPassBoundsMode::Unchecked,
                BoundsMode::Checked => MirPassBoundsMode::Checked,
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
            bounds_mode,
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
fn c_backend_should_emit_and_run_unchecked_void_functions() {
    let c = emit_c(
        r#"
      fn set_first(out: ptr<i32>) -> void { out[0] = 41; }
      export fn mutate(out: ptr<i32>, stop: bool) -> void {
        if stop { return; }
        set_first(out);
        out[0] = out[0] + 1;
      }
    "#,
    );
    assert!(c.contains("static void set_first(int32_t* out)"), "{c}");
    assert!(c.contains("void mutate(int32_t* out, bool stop)"), "{c}");
    assert!(c.contains("set_first(out);"), "{c}");
    assert!(c.contains("return;"), "{c}");

    let harness = format!(
        r#"
{c}
int main(void) {{
  int32_t value = 0;
  mutate(&value, false);
  if (value != 42) return 1;
  mutate(&value, true);
  if (value != 42) return 2;
  return 0;
}}
"#
    );
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("rust_calckernel_void_c_{unique}"));
    fs::create_dir_all(&dir).expect("create temp dir");
    let source = dir.join("void.c");
    let binary = dir.join("void");
    fs::write(&source, harness).expect("write harness");
    let compile = Command::new("clang")
        .arg("-std=c11")
        .arg("-Wall")
        .arg("-Wextra")
        .arg("-Werror")
        .arg(&source)
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
fn c_backend_should_emit_status_void_without_ck_return() {
    let c = emit_checked_c(
        r#"
      fn no_op() -> void { return; }
      export fn run() -> void { no_op(); }
    "#,
    );

    assert!(c.contains("static CK_Status no_op()"), "{c}");
    assert!(c.contains("CK_Status run()"), "{c}");
    assert!(!c.contains("ck_return"), "{c}");
    assert!(c.contains("return CK_OK;"), "{c}");
}

#[test]
fn c_backend_should_propagate_checked_void_call_failures() {
    let c = emit_checked_c(
        r#"
      fn increment(out: ptr<i64>) -> void { out[0] = out[0] + 1; }
      fn middle(out: ptr<i64>) -> void { increment(out); }
      export fn run(out: ptr<i64>) -> void { middle(out); }
    "#,
    );
    assert!(c.contains("ik_status = increment(out);"), "{c}");
    assert!(c.contains("ik_status = middle(out);"), "{c}");

    let harness = format!(
        r#"
{c}
int main(void) {{
  int64_t value = 1;
  if (run(&value) != CK_OK || value != 2) return 1;
  value = INT64_MAX;
  if (run(&value) != CK_ERR_OVERFLOW) return 2;
  return 0;
}}
"#
    );
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("rust_calckernel_void_checked_c_{unique}"));
    fs::create_dir_all(&dir).expect("create temp dir");
    let source = dir.join("void_checked.c");
    let binary = dir.join("void_checked");
    fs::write(&source, harness).expect("write harness");
    let compile = Command::new("clang")
        .arg("-std=c11")
        .arg("-Wall")
        .arg("-Wextra")
        .arg("-Werror")
        .arg(&source)
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
    let examples = fixtures::ORACLE_EXAMPLES
        .iter()
        .map(|fixture| fixture.oracle)
        .chain(std::iter::once("tests/fixtures/f64_edges.ck"))
        .collect::<Vec<_>>();

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
            fixtures::BENCH_PRICING_HELPERS.oracle,
            "-O0",
        ),
        (
            "pricing_helpers_o2",
            fixtures::BENCH_PRICING_HELPERS.oracle,
            "-O2",
        ),
        ("pricing_soa_o3", fixtures::BENCH_PRICING_SOA.oracle, "-O3"),
        ("f64_kernels_o3", fixtures::BENCH_F64_KERNELS.oracle, "-O3"),
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
            String::from_utf8(rust_output.stdout)
                .expect("Rust stdout should be UTF-8")
                .replace(", bounds=unchecked", ""),
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

#[test]
fn c_backend_should_emit_dependency_ordered_slice_descriptors() {
    let c = emit_c(
        r#"
      struct Holder {
        node: Node;
        nodes: slice<Node>;
      }

      struct Node {
        value: i32;
      }

      export fn count(holder: ptr<Holder>) -> u32 {
        return holder[0].nodes.len;
      }
    "#,
    );

    let holder_forward = c
        .find("typedef struct Holder Holder;")
        .expect("Holder forward");
    let node_forward = c.find("typedef struct Node Node;").expect("Node forward");
    let descriptor = c
        .find("typedef struct CK_Slice_Node {")
        .expect("slice descriptor");
    let node_definition = c.find("struct Node {").expect("Node definition");
    let holder_definition = c.find("struct Holder {").expect("Holder definition");

    assert!(holder_forward < descriptor);
    assert!(node_forward < descriptor);
    assert!(descriptor < node_definition);
    assert!(node_definition < holder_definition);
    assert!(c.contains("Node* data;"));
    assert!(c.contains("uint32_t len;"));
}

#[test]
fn c_backend_should_flatten_exported_and_internal_slice_params() {
    let c = emit_c(
        r#"
      fn first(items: slice<i32>) -> i32 {
        return items[0];
      }

      export fn exported_first(items: slice<i32>) -> i32 {
        return first(items);
      }
    "#,
    );

    assert!(c.contains("static int32_t first(int32_t* items_data, uint32_t items_len)"));
    assert!(c.contains("int32_t exported_first(int32_t* items_data, uint32_t items_len)"));
    assert!(c.contains("CK_Slice_i32 items;"));
    assert!(c.contains("items.data = items_data;"));
    assert!(c.contains("items.len = items_len;"));
    assert!(c.contains("first(items.data, items.len)"));
}

#[test]
fn c_backend_should_copy_slice_locals_fields_and_internal_returns() {
    let c = emit_c(
        r#"
      struct Holder {
        items: slice<i32>;
      }

      fn identity(items: slice<i32>) -> slice<i32> {
        let copy: slice<i32> = items;
        return copy;
      }

      export fn round_trip(holder: ptr<Holder>, data: ptr<i32>, len: u32) -> i32 {
        holder[0].items = slice(data, len);
        let returned: slice<i32> = identity(holder[0].items);
        return returned[0];
      }
    "#,
    );

    assert!(c.contains("static CK_Slice_i32 identity(int32_t* items_data, uint32_t items_len)"));
    assert!(c.contains("copy = items;"));
    assert!(c.contains("return copy;"));
    assert!(c.contains("identity("));

    let harness = format!(
        r#"
{c}

int main(void) {{
  int32_t values[1] = {{42}};
  Holder holder = {{0}};
  return round_trip(&holder, values, 1) == 42 ? 0 : 1;
}}
"#
    );
    compile_and_run_c(&harness, "slice_copy_return");
}

#[test]
fn c_backend_should_disambiguate_generated_slice_and_parameter_names() {
    let source = r#"
      struct CK_Slice_i32 {
        marker: i32;
      }

      fn helper(value: i32) -> i32 {
        return value;
      }

      export fn collide(
        items: slice<i32>,
        items_data: ptr<i32>,
        items_len: u32,
        ck_return: ptr<i32>,
        ik_status: i32,
        ik_tmp0: i32
      ) -> i32 {
        return helper(items[0] + items_data[0] + ik_status + ik_tmp0);
      }
    "#;
    let c = emit_c(source);

    assert!(c.contains("typedef struct CK_Slice_i32_1 {"));
    assert!(c.contains("int32_t* items_data_1, uint32_t items_len_1"));
    assert!(c.contains("CK_Slice_i32_1 items;"));
    assert!(c.contains("items.data = items_data_1;"));
    assert!(c.contains("items.len = items_len_1;"));

    let checked =
        emit_c_with_modes_and_opt_level(source, OverflowMode::Unchecked, BoundsMode::Checked, 1);
    assert!(checked.contains("int32_t* ck_return_1"), "{checked}");
    assert!(checked.contains("CK_Status ik_status_1;"), "{checked}");
    assert!(checked.contains("ik_tmp0_1"), "{checked}");
}

#[test]
fn c_backend_should_compile_generated_slice_headers_with_werror() {
    let source = SourceFile::new(
        "test.ck",
        r#"
      struct Holder {
        items: slice<i32>;
      }

      export fn store_first(holder: ptr<Holder>, items: slice<i32>) -> void {
        holder[0].items = items;
      }
    "#,
    );
    let checked = check(&source);
    assert_eq!(checked.diagnostics, []);
    let mir = lower_to_mir(&checked.checked_program).expect("MIR lowering should succeed");
    let options = EmitCOptions {
        overflow_mode: OverflowMode::Unchecked,
        bounds_mode: BoundsMode::Unchecked,
        opt_level: 0,
    };
    let header = emit_c_header(&mir, options);
    let implementation = emit_c_module_with_header(&mir, options, "kernel.h");

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("rust_calckernel_slice_header_{unique}"));
    fs::create_dir_all(&dir).expect("create temp dir");
    fs::write(dir.join("kernel.h"), header).expect("write header");
    fs::write(dir.join("kernel.c"), implementation).expect("write source");
    let compile = Command::new("clang")
        .arg("-std=c11")
        .arg("-Wall")
        .arg("-Wextra")
        .arg("-Werror")
        .arg("-c")
        .arg("kernel.c")
        .current_dir(&dir)
        .output()
        .expect("run clang");
    assert!(
        compile.status.success(),
        "clang failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&compile.stdout),
        String::from_utf8_lossy(&compile.stderr)
    );
}

#[test]
fn checked_c_backend_should_guard_slice_reads_writes_and_nested_fields() {
    let source = r#"
      struct Item {
        value: i32;
      }

      export fn bump(items: slice<Item>, index: u32) -> i32 {
        items[index].value = items[index].value + 1;
        return items[index].value;
      }
    "#;
    for opt_level in 0..=3 {
        let c = emit_c_with_modes_and_opt_level(
            source,
            OverflowMode::Unchecked,
            BoundsMode::Checked,
            opt_level,
        );
        assert_eq!(c.matches(">= items.len").count(), 3, "{c}");
        assert_eq!(c.matches("return CK_ERR_OUT_OF_BOUNDS;").count(), 3, "{c}");
        assert!(c.contains("items.data["));
        assert!(c.contains("].value"));
        let harness = format!(
            r#"
{c}
int main(void) {{
  Item items[1] = {{{{41}}}};
  int32_t result = 0;
  if (bump(items, 1, 0, &result) != CK_OK || result != 42) return 1;
  if (bump(items, 1, 1, &result) != CK_ERR_OUT_OF_BOUNDS) return 2;
  return 0;
}}
"#
        );
        compile_and_run_c(&harness, &format!("slice_nested_field_o{opt_level}"));
    }
}

#[test]
fn checked_c_backend_should_guard_subslice_before_arithmetic_or_pointer_advance() {
    let source = r#"
      export fn range_len(items: slice<i32>, start: u32, end: u32) -> u32 {
        let middle: slice<i32> = items[start..end];
        return middle.len;
      }
    "#;
    for opt_level in 0..=3 {
        let c = emit_c_with_modes_and_opt_level(
            source,
            OverflowMode::Unchecked,
            BoundsMode::Checked,
            opt_level,
        );
        let guard = c.find(" > items.len").expect("subslice guard");
        let pointer_advance = c.find("items.data +").expect("pointer advance");
        let subtraction = c.find(" - ").expect("range subtraction");
        assert!(guard < pointer_advance, "{c}");
        assert!(guard < subtraction, "{c}");
    }
}

#[test]
fn checked_c_backend_should_return_out_of_bounds_for_edge_cases() {
    let source = r#"
      export fn read_at(items: slice<i32>, index: u32) -> i32 {
        return items[index];
      }

      export fn range_len(items: slice<i32>, start: u32, end: u32) -> u32 {
        let middle: slice<i32> = items[start..end];
        return middle.len;
      }
    "#;

    for opt_level in 0..=3 {
        let c = emit_c_with_modes_and_opt_level(
            source,
            OverflowMode::Unchecked,
            BoundsMode::Checked,
            opt_level,
        );
        for declaration in [
            "#define CK_OK ((CK_Status)0)",
            "#define CK_ERR_OVERFLOW ((CK_Status)1)",
            "#define CK_ERR_DIV_BY_ZERO ((CK_Status)2)",
            "#define CK_ERR_NULL_POINTER ((CK_Status)3)",
            "#define CK_ERR_OUT_OF_BOUNDS ((CK_Status)4)",
        ] {
            assert!(
                c.contains(declaration),
                "O{opt_level}: missing {declaration}"
            );
        }
        let harness = format!(
            r#"
{c}

int main(void) {{
  int32_t values[2] = {{11, 22}};
  int32_t value = 0;
  uint32_t len = 99;
  if (read_at(values, 2, 1, &value) != CK_OK || value != 22) return 1;
  if (read_at(values, 2, 2, &value) != CK_ERR_OUT_OF_BOUNDS) return 2;
  if (read_at(values, 2, UINT32_MAX, &value) != CK_ERR_OUT_OF_BOUNDS) return 3;
  if (range_len(values, 2, 2, 1, &len) != CK_ERR_OUT_OF_BOUNDS) return 4;
  if (range_len(values, 2, 0, 3, &len) != CK_ERR_OUT_OF_BOUNDS) return 5;
  if (range_len(values, 2, 1, 1, &len) != CK_OK || len != 0) return 6;
  if (read_at(values, 2, 0, NULL) != CK_ERR_NULL_POINTER) return 7;
  return 0;
}}
"#
        );
        compile_and_run_c(&harness, &format!("slice_bounds_edges_o{opt_level}"));
    }
}

#[test]
fn checked_c_backend_should_preserve_empty_zero_start_slice_pointer() {
    let source = r#"
      export fn empty_data(items: slice<i32>) -> ptr<i32> {
        let empty: slice<i32> = items[0..0];
        return empty.data;
      }
    "#;
    for opt_level in 0..=3 {
        let c = emit_c_with_modes_and_opt_level(
            source,
            OverflowMode::Unchecked,
            BoundsMode::Checked,
            opt_level,
        );
        assert!(c.contains("== 0 ? items.data : items.data +"), "{c}");
        let harness = format!(
            r#"
{c}

int main(void) {{
  int32_t values[1] = {{7}};
  int32_t* result = NULL;
  if (empty_data(values, 0, &result) != CK_OK) return 1;
  return result == values ? 0 : 2;
}}
"#
        );
        compile_and_run_c(&harness, &format!("slice_zero_start_pointer_o{opt_level}"));
    }
}

#[test]
fn checked_c_backend_should_propagate_bounds_through_void_value_and_slice_calls() {
    let source = r#"
      fn touch(items: slice<i32>, index: u32) -> void {
        items[index] = items[index] + 1;
      }

      fn narrow(items: slice<i32>, end: u32) -> slice<i32> {
        return items[0..end];
      }

      fn read(items: slice<i32>, index: u32) -> i32 {
        return items[index];
      }

      export fn dispatch(items: slice<i32>, index: u32) -> i32 {
        touch(items, index);
        let head: slice<i32> = narrow(items, index + 1);
        return read(head, index);
      }
    "#;
    for opt_level in 0..=3 {
        let c = emit_c_with_modes_and_opt_level(
            source,
            OverflowMode::Unchecked,
            BoundsMode::Checked,
            opt_level,
        );
        assert!(c.contains("static CK_Status touch("));
        assert!(c.contains("static CK_Status narrow("));
        assert!(c.contains("CK_Slice_i32* ck_return"));
        assert!(c.contains("return ik_status;"));
        let harness = format!(
            r#"
{c}

int main(void) {{
  int32_t values[2] = {{10, 20}};
  int32_t result = 0;
  if (dispatch(values, 2, 1, &result) != CK_OK || result != 21) return 1;
  if (dispatch(values, 2, 2, &result) != CK_ERR_OUT_OF_BOUNDS) return 2;
  return 0;
}}
"#
        );
        compile_and_run_c(&harness, &format!("slice_call_status_o{opt_level}"));
    }
}

#[test]
fn checked_c_backend_should_preserve_overflow_call_and_bounds_error_precedence() {
    let source = r#"
      fn quotient(value: u32, divisor: u32) -> u32 {
        return value / divisor;
      }

      export fn added_index(items: slice<i32>, base: u32, offset: u32) -> i32 {
        return items[base + offset];
      }

      export fn called_index(items: slice<i32>, value: u32, divisor: u32) -> i32 {
        return items[quotient(value, divisor)];
      }
    "#;
    for opt_level in 0..=3 {
        let c = emit_c_with_modes_and_opt_level(
            source,
            OverflowMode::Checked,
            BoundsMode::Checked,
            opt_level,
        );
        let harness = format!(
            r#"
{c}

int main(void) {{
  int32_t values[1] = {{5}};
  int32_t result = 0;
  if (added_index(values, 1, UINT32_MAX, 1, &result) != CK_ERR_OVERFLOW) return 1;
  if (called_index(values, 1, 10, 0, &result) != CK_ERR_DIV_BY_ZERO) return 2;
  return 0;
}}
"#
        );
        compile_and_run_c(&harness, &format!("slice_error_precedence_o{opt_level}"));
    }
}

#[test]
fn checked_c_backend_should_leave_raw_pointer_and_slice_data_access_unchecked() {
    let source = r#"
      export fn raw_read(data: ptr<i32>, index: u32) -> i32 {
        return data[index];
      }

      export fn escaped_read(items: slice<i32>, index: u32) -> i32 {
        return items.data[index];
      }
    "#;
    for opt_level in 0..=3 {
        let c = emit_c_with_modes_and_opt_level(
            source,
            OverflowMode::Unchecked,
            BoundsMode::Checked,
            opt_level,
        );
        assert!(c.contains("#define CK_ERR_OUT_OF_BOUNDS"));
        assert!(!c.contains(">= items.len"), "{c}");
        assert_eq!(c.matches("return CK_ERR_OUT_OF_BOUNDS;").count(), 0, "{c}");
    }
}

#[test]
fn checked_c_backend_should_preserve_guards_at_o0_through_o3() {
    let source = r#"
      export fn read_range(items: slice<i32>, index: u32, end: u32) -> i32 {
        let head: slice<i32> = items[0..end];
        return head[index];
      }
    "#;
    for opt_level in 0..=3 {
        let c = emit_c_with_modes_and_opt_level(
            source,
            OverflowMode::Unchecked,
            BoundsMode::Checked,
            opt_level,
        );
        assert_eq!(
            c.matches("return CK_ERR_OUT_OF_BOUNDS;").count(),
            2,
            "O{opt_level}:\n{c}"
        );
        assert!(c.contains(" > items.len"), "O{opt_level}:\n{c}");
        assert!(c.contains(">= head.len"), "O{opt_level}:\n{c}");
    }
}

fn compile_and_run_c(source: &str, case_name: &str) {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("rust_calckernel_{case_name}_{unique}"));
    fs::create_dir_all(&dir).expect("create temp dir");
    let c_path = dir.join("harness.c");
    let bin_path = dir.join("harness");
    fs::write(&c_path, source).expect("write harness");

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
        "harness failed with {:?}\nstdout:\n{}\nstderr:\n{}",
        run.status.code(),
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
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
