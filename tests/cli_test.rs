use std::{
    ffi::OsString,
    fs,
    path::PathBuf,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

#[path = "support/fixtures.rs"]
mod fixtures;

#[test]
fn cli_should_check_and_emit_core_outputs() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("rust_calckernel_cli_{unique}"));
    fs::create_dir_all(&dir).expect("create temp dir");
    let source = dir.join("sample.ck");
    fs::write(
        &source,
        r#"
          export fn add_i64(a: i64, b: i64) -> i64 {
            return a + b;
          }
        "#,
    )
    .expect("write source");

    let bin = env!("CARGO_BIN_EXE_ckc");

    let check = Command::new(bin)
        .arg("check")
        .arg(&source)
        .output()
        .expect("run check");
    assert!(check.status.success());
    assert!(String::from_utf8_lossy(&check.stdout).contains("OK:"));

    let mir = Command::new(bin)
        .arg("emit-mir")
        .arg(&source)
        .output()
        .expect("run emit-mir");
    assert!(mir.status.success());
    assert!(String::from_utf8_lossy(&mir.stdout).contains("export fn add_i64"));

    let c_path = dir.join("sample.c");
    let emit_c = Command::new(bin)
        .arg("emit-c")
        .arg("--out")
        .arg(&c_path)
        .arg(&source)
        .output()
        .expect("run emit-c");
    assert!(emit_c.status.success());
    assert!(
        fs::read_to_string(&c_path)
            .expect("read c")
            .contains("int64_t add_i64")
    );
    assert!(
        fs::read_to_string(dir.join("sample.h"))
            .expect("read default header")
            .contains("CK_API int64_t add_i64")
    );

    let checked_c_path = dir.join("checked.c");
    let checked_header_path = dir.join("api.h");
    let emit_checked_c = Command::new(bin)
        .arg("emit-c")
        .arg("--overflow")
        .arg("checked")
        .arg("--out")
        .arg(&checked_c_path)
        .arg("--header")
        .arg(&checked_header_path)
        .arg(&source)
        .output()
        .expect("run checked emit-c");
    assert!(
        emit_checked_c.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&emit_checked_c.stderr)
    );
    assert!(
        fs::read_to_string(&checked_c_path)
            .expect("read checked c")
            .contains("#include \"api.h\"")
    );
    assert!(
        fs::read_to_string(&checked_header_path)
            .expect("read checked header")
            .contains("CK_API CK_Status add_i64")
    );

    let wat = Command::new(bin)
        .arg("emit-wat")
        .arg(&source)
        .output()
        .expect("run emit-wat");
    assert!(wat.status.success());
    assert!(String::from_utf8_lossy(&wat.stdout).contains("(func $add_i64"));

    let llvm = Command::new(bin)
        .arg("emit-llvm")
        .arg(&source)
        .output()
        .expect("run emit-llvm");
    assert!(llvm.status.success());
    assert!(String::from_utf8_lossy(&llvm.stdout).contains("define i64 @add_i64"));

    let wasm_path = dir.join("sample.wasm");
    let wasm = Command::new(bin)
        .arg("emit-wasm")
        .arg("--out")
        .arg(&wasm_path)
        .arg(&source)
        .output()
        .expect("run emit-wasm");
    assert!(
        wasm.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&wasm.stderr)
    );
    let wasm_bytes = fs::read(&wasm_path).expect("read wasm");
    assert_eq!(&wasm_bytes[..4], b"\0asm");
    assert_eq!(&wasm_bytes[4..8], &[1, 0, 0, 0]);
}

#[test]
fn cli_should_build_native_c_library_and_llvm_object() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("rust_calckernel_cli_build_{unique}"));
    fs::create_dir_all(&dir).expect("create temp dir");
    let source = dir.join("sample.ck");
    fs::write(
        &source,
        "export fn add_i64(a: i64, b: i64) -> i64 { return a + b; }",
    )
    .expect("write source");

    let bin = env!("CARGO_BIN_EXE_ckc");

    let library_base = dir.join("libsample");
    let build = Command::new(bin)
        .arg("build")
        .arg("--out")
        .arg(&library_base)
        .arg(&source)
        .output()
        .expect("run build");
    assert!(
        build.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&build.stderr)
    );
    assert!(shared_library_path(&library_base).exists());
    assert!(library_base.with_extension("c").exists());
    assert!(library_base.with_extension("h").exists());

    let checked_library_base = dir.join("libsample_checked");
    let build_checked = Command::new(bin)
        .arg("build")
        .arg("--overflow")
        .arg("checked")
        .arg("--out")
        .arg(&checked_library_base)
        .arg(&source)
        .output()
        .expect("run checked build");
    assert!(
        build_checked.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&build_checked.stderr)
    );
    assert!(shared_library_path(&checked_library_base).exists());
    assert!(checked_library_base.with_extension("h").exists());
    assert!(
        fs::read_to_string(checked_library_base.with_extension("c"))
            .expect("read checked c")
            .contains("#include \"libsample_checked.h\"")
    );
    assert!(
        fs::read_to_string(checked_library_base.with_extension("h"))
            .expect("read checked header")
            .contains("CK_Status add_i64")
    );

    let object_path = dir.join("sample.o");
    let build_llvm = Command::new(bin)
        .arg("build-llvm")
        .arg("--kind")
        .arg("object")
        .arg("--out")
        .arg(&object_path)
        .arg(&source)
        .output()
        .expect("run build-llvm");
    assert!(
        build_llvm.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&build_llvm.stderr)
    );
    assert!(object_path.exists());
    assert!(object_path.with_extension("ll").exists());
}

#[test]
fn emit_c_should_not_leave_c_output_when_header_path_cannot_be_created() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("rust_calckernel_emit_c_failure_{unique}"));
    fs::create_dir_all(&dir).expect("create temp dir");
    let source = dir.join("sample.ck");
    fs::write(
        &source,
        "export fn add_i64(a: i64, b: i64) -> i64 { return a + b; }",
    )
    .expect("write source");
    let c_path = dir.join("build").join("sample.c");
    let header_parent = dir.join("headers");
    fs::write(&header_parent, "not a directory").expect("write header parent file");
    let header_path = header_parent.join("sample.h");

    let output = Command::new(env!("CARGO_BIN_EXE_ckc"))
        .arg("emit-c")
        .arg("--out")
        .arg(&c_path)
        .arg("--header")
        .arg(&header_path)
        .arg(&source)
        .output()
        .expect("run emit-c with invalid header path");
    assert!(!output.status.success());
    assert!(
        !c_path.exists(),
        "emit-c must not leave C output when header creation fails\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!header_path.exists());
}

#[cfg(unix)]
#[test]
fn build_should_pass_typescript_compatible_ck_build_dll_define_to_clang() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("rust_calckernel_fake_clang_{unique}"));
    fs::create_dir_all(&dir).expect("create temp dir");
    let fake_bin_dir = dir.join("bin");
    fs::create_dir_all(&fake_bin_dir).expect("create fake bin dir");
    let log_path = dir.join("clang-args.log");
    let fake_clang = fake_bin_dir.join("clang");
    fs::write(
        &fake_clang,
        r#"#!/bin/sh
printf '%s\n' "$*" >> "$CKC_CLANG_LOG"
if [ "$1" = "--version" ]; then
  echo "fake clang"
  exit 0
fi
out=""
previous=""
for arg in "$@"; do
  if [ "$previous" = "-o" ]; then
    out="$arg"
  fi
  previous="$arg"
done
if [ -n "$out" ]; then
  mkdir -p "$(dirname "$out")"
  : > "$out"
fi
exit 0
"#,
    )
    .expect("write fake clang");
    fs::set_permissions(&fake_clang, fs::Permissions::from_mode(0o755)).expect("chmod fake clang");

    let source = dir.join("sample.ck");
    fs::write(
        &source,
        "export fn add_i64(a: i64, b: i64) -> i64 { return a + b; }",
    )
    .expect("write source");
    let original_path = std::env::var_os("PATH").unwrap_or_default();
    let mut path_entries = vec![fake_bin_dir];
    path_entries.extend(std::env::split_paths(&original_path));
    let path = std::env::join_paths(path_entries).expect("join PATH");

    let library_base = dir.join("libsample");
    let output = Command::new(env!("CARGO_BIN_EXE_ckc"))
        .env("PATH", path)
        .env("CKC_CLANG_LOG", &log_path)
        .arg("build")
        .arg("--out")
        .arg(&library_base)
        .arg(&source)
        .output()
        .expect("run build with fake clang");
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let log = fs::read_to_string(&log_path).expect("read clang args log");
    let compile_args = log
        .lines()
        .find(|line| line.contains("-shared"))
        .unwrap_or_else(|| panic!("missing compile clang invocation:\n{log}"));
    assert!(
        compile_args.contains("-DCK_BUILD_DLL"),
        "clang invocation must match TypeScript build macro contract:\n{log}"
    );
}

#[test]
fn build_llvm_should_print_typescript_compatible_missing_clang_message() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("rust_calckernel_missing_clang_{unique}"));
    fs::create_dir_all(&dir).expect("create temp dir");
    let empty_path = dir.join("empty_path");
    fs::create_dir_all(&empty_path).expect("create empty PATH dir");
    let source = dir.join("sample.ck");
    fs::write(
        &source,
        "export fn add_i64(a: i64, b: i64) -> i64 { return a + b; }",
    )
    .expect("write source");

    let output = Command::new(env!("CARGO_BIN_EXE_ckc"))
        .arg("build-llvm")
        .arg("--out")
        .arg(dir.join("libsample"))
        .arg(&source)
        .env("PATH", &empty_path)
        .output()
        .expect("run build-llvm");

    assert!(!output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr
            .contains("clang was not found. Install clang and make sure it is available on PATH.")
    );
    assert!(stderr.contains("You can still run emit-llvm to generate LLVM IR without clang."));
}

#[test]
fn build_should_match_typescript_oracle_for_generated_c_header_and_stdout() {
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
    let dir = std::env::temp_dir().join(format!("rust_calckernel_build_oracle_{unique}"));
    fs::create_dir_all(&dir).expect("create temp dir");
    let source = typescript_root().join(fixtures::CORE_SCALAR.oracle);

    for overflow in ["unchecked", "checked"] {
        let output_base = dir.join(format!("libscalar_{overflow}"));
        let args = vec![
            os("build"),
            os("--out"),
            output_base.clone().into_os_string(),
            os("--overflow"),
            os(overflow),
            source.clone().into_os_string(),
        ];

        let ts = run_typescript_cli(&ts_cli, &args);
        assert_eq!(ts.status_code, Some(0), "TS stderr: {}", ts.stderr);
        let ts_c = fs::read_to_string(format!("{}.c", output_base.display())).expect("read TS C");
        let ts_h = fs::read_to_string(format!("{}.h", output_base.display())).expect("read TS H");

        let rust = run_rust_cli(&args);
        assert_eq!(rust.status_code, ts.status_code, "overflow={overflow}");
        assert_eq!(
            legacy_cli_text(&rust.stdout),
            ts.stdout,
            "overflow={overflow} stdout"
        );
        assert_eq!(rust.stderr, ts.stderr, "overflow={overflow} stderr");
        assert_eq!(
            fs::read_to_string(format!("{}.c", output_base.display())).expect("read Rust C"),
            ts_c,
            "overflow={overflow} C"
        );
        assert_eq!(
            fs::read_to_string(format!("{}.h", output_base.display()))
                .expect("read Rust H")
                .replace("#define CK_ERR_OUT_OF_BOUNDS ((CK_Status)4)\n", ""),
            ts_h,
            "overflow={overflow} H"
        );
    }
}

#[test]
fn build_should_match_typescript_oracle_for_pricing_dynamic_library_runtime_behavior() {
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
    let dir = std::env::temp_dir().join(format!("rust_calckernel_build_runtime_{unique}"));
    fs::create_dir_all(&dir).expect("create temp dir");
    let runner = dir.join("run_pricing_dynamic.py");
    fs::write(&runner, pricing_dynamic_library_runner()).expect("write dynamic library runner");
    let source = typescript_root().join(fixtures::APPLICATION_PRICING.oracle);

    for overflow in ["unchecked", "checked"] {
        let ts_base = dir.join(format!("ts_pricing_{overflow}"));
        let rust_base = dir.join(format!("rust_pricing_{overflow}"));
        let ts_args = vec![
            os("build"),
            os("--out"),
            ts_base.clone().into_os_string(),
            os("--overflow"),
            os(overflow),
            source.clone().into_os_string(),
        ];
        let rust_args = vec![
            os("build"),
            os("--out"),
            rust_base.clone().into_os_string(),
            os("--overflow"),
            os(overflow),
            source.clone().into_os_string(),
        ];

        let ts_build = run_typescript_cli(&ts_cli, &ts_args);
        assert_eq!(
            ts_build.status_code,
            Some(0),
            "{overflow} TS stderr: {}",
            ts_build.stderr
        );
        let rust_build = run_rust_cli(&rust_args);
        assert_eq!(
            rust_build.status_code, ts_build.status_code,
            "{overflow} build status"
        );

        let mode = if overflow == "checked" {
            "checked"
        } else {
            "unchecked"
        };
        let ts_run =
            run_python_pricing_dynamic_library(&runner, mode, &shared_library_path(&ts_base));
        let rust_run =
            run_python_pricing_dynamic_library(&runner, mode, &shared_library_path(&rust_base));

        assert_eq!(
            rust_run.status_code, ts_run.status_code,
            "{overflow} runtime status"
        );
        assert_eq!(rust_run.stdout, ts_run.stdout, "{overflow} runtime stdout");
        assert_eq!(rust_run.stderr, ts_run.stderr, "{overflow} runtime stderr");
    }
}

#[test]
fn build_should_match_typescript_oracle_for_official_c_dynamic_library_runtime_behavior() {
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
    let dir =
        std::env::temp_dir().join(format!("rust_calckernel_build_official_c_runtime_{unique}"));
    fs::create_dir_all(&dir).expect("create temp dir");
    let runner = dir.join("run_official_c_dynamic.py");
    fs::write(&runner, official_c_dynamic_library_runner())
        .expect("write official C dynamic library runner");
    let cases = [
        ("c-scalar", "unchecked", fixtures::CORE_SCALAR.oracle),
        ("c-casts", "unchecked", fixtures::CORE_EXPLICIT_CASTS.oracle),
        (
            "c-dijkstra",
            "unchecked",
            fixtures::APPLICATION_DIJKSTRA.oracle,
        ),
        ("c-f64-array", "unchecked", fixtures::WASM_F64_ARRAY.oracle),
        ("c-f64-axpy", "unchecked", fixtures::WASM_F64_AXPY.oracle),
        ("c-f64-sum", "unchecked", fixtures::WASM_F64_SUM.oracle),
        (
            "c-pricing-soa",
            "unchecked",
            fixtures::WASM_PRICING_SOA.oracle,
        ),
        ("c-wasm-scalar", "unchecked", fixtures::WASM_SCALAR.oracle),
        ("c-wasm-calls", "unchecked", fixtures::WASM_CALLS.oracle),
        (
            "c-wasm-control-flow",
            "unchecked",
            fixtures::WASM_CONTROL_FLOW.oracle,
        ),
        ("c-wasm-memory", "unchecked", fixtures::WASM_MEMORY.oracle),
        (
            "c-wasm-short-circuit",
            "unchecked",
            fixtures::WASM_SHORT_CIRCUIT.oracle,
        ),
        ("c-llvm-scalar", "unchecked", fixtures::LLVM_SCALAR.oracle),
        ("c-llvm-calls", "unchecked", fixtures::LLVM_CALLS.oracle),
        (
            "c-llvm-control-flow",
            "unchecked",
            fixtures::LLVM_CONTROL_FLOW.oracle,
        ),
        ("c-llvm-memory", "unchecked", fixtures::LLVM_MEMORY.oracle),
        (
            "c-llvm-short-circuit",
            "unchecked",
            fixtures::LLVM_SHORT_CIRCUIT.oracle,
        ),
        ("c-llvm-bool", "unchecked", fixtures::LLVM_BOOL.oracle),
        (
            "c-scalar-checked",
            "checked",
            fixtures::CHECKED_SCALAR.oracle,
        ),
        (
            "c-control-checked",
            "checked",
            fixtures::CHECKED_SCALAR_CONTROL.oracle,
        ),
        (
            "c-logical-checked",
            "checked",
            fixtures::CHECKED_SCALAR_LOGICAL.oracle,
        ),
        (
            "c-calls-checked",
            "checked",
            fixtures::CHECKED_SCALAR_CALLS.oracle,
        ),
    ];

    for (case_name, overflow, example) in cases {
        let source = typescript_root().join(example);
        let ts_base = dir.join(format!("ts_{case_name}"));
        let rust_base = dir.join(format!("rust_{case_name}"));
        let ts_args = vec![
            os("build"),
            os("--out"),
            ts_base.clone().into_os_string(),
            os("--overflow"),
            os(overflow),
            source.clone().into_os_string(),
        ];
        let rust_args = vec![
            os("build"),
            os("--out"),
            rust_base.clone().into_os_string(),
            os("--overflow"),
            os(overflow),
            source.clone().into_os_string(),
        ];

        let ts_build = run_typescript_cli(&ts_cli, &ts_args);
        assert_eq!(
            ts_build.status_code,
            Some(0),
            "{case_name} TS stderr: {}",
            ts_build.stderr
        );
        let rust_build = run_rust_cli(&rust_args);
        assert_eq!(
            rust_build.status_code, ts_build.status_code,
            "{case_name} build status"
        );

        let ts_run = run_python_official_c_dynamic_library(
            &runner,
            case_name,
            &shared_library_path(&ts_base),
        );
        let rust_run = run_python_official_c_dynamic_library(
            &runner,
            case_name,
            &shared_library_path(&rust_base),
        );

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
        assert_eq!(
            rust_run.status_code, ts_run.status_code,
            "{case_name} runtime status"
        );
        assert_eq!(rust_run.stdout, ts_run.stdout, "{case_name} runtime stdout");
        assert_eq!(rust_run.stderr, ts_run.stderr, "{case_name} runtime stderr");
    }
}

#[test]
fn build_should_match_typescript_oracle_for_perf_c_dynamic_library_runtime_behavior() {
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
    let dir = std::env::temp_dir().join(format!("rust_calckernel_build_perf_c_runtime_{unique}"));
    fs::create_dir_all(&dir).expect("create temp dir");
    let runner = dir.join("run_perf_c_dynamic.py");
    fs::write(&runner, official_c_dynamic_library_runner())
        .expect("write perf C dynamic library runner");
    let cases = [
        (
            "c-perf-pricing-helpers-o0",
            fixtures::BENCH_PRICING_HELPERS.oracle,
            "-O0",
        ),
        (
            "c-perf-pricing-helpers-o2",
            fixtures::BENCH_PRICING_HELPERS.oracle,
            "-O2",
        ),
        (
            "c-perf-pricing-soa-o3",
            fixtures::BENCH_PRICING_SOA.oracle,
            "-O3",
        ),
        (
            "c-perf-f64-kernels-o3",
            fixtures::BENCH_F64_KERNELS.oracle,
            "-O3",
        ),
    ];

    for (case_name, fixture, opt_level) in cases {
        let source = typescript_root().join(fixture);
        let ts_base = dir.join(format!("ts_{case_name}"));
        let rust_base = dir.join(format!("rust_{case_name}"));
        let ts_args = vec![
            os("build"),
            os("--out"),
            ts_base.clone().into_os_string(),
            os("--overflow"),
            os("unchecked"),
            os(opt_level),
            source.clone().into_os_string(),
        ];
        let rust_args = vec![
            os("build"),
            os("--out"),
            rust_base.clone().into_os_string(),
            os("--overflow"),
            os("unchecked"),
            os(opt_level),
            source.clone().into_os_string(),
        ];

        let ts_build = run_typescript_cli(&ts_cli, &ts_args);
        assert_eq!(
            ts_build.status_code,
            Some(0),
            "{case_name} TS stderr: {}",
            ts_build.stderr
        );
        let rust_build = run_rust_cli(&rust_args);
        assert_eq!(
            rust_build.status_code, ts_build.status_code,
            "{case_name} build status"
        );

        let ts_run = run_python_official_c_dynamic_library(
            &runner,
            case_name,
            &shared_library_path(&ts_base),
        );
        let rust_run = run_python_official_c_dynamic_library(
            &runner,
            case_name,
            &shared_library_path(&rust_base),
        );

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
        assert_eq!(
            rust_run.status_code, ts_run.status_code,
            "{case_name} runtime status"
        );
        assert_eq!(rust_run.stdout, ts_run.stdout, "{case_name} runtime stdout");
        assert_eq!(rust_run.stderr, ts_run.stderr, "{case_name} runtime stderr");
    }
}

#[test]
fn build_should_match_typescript_oracle_for_f64_edge_c_dynamic_library_runtime_behavior() {
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
    let dir =
        std::env::temp_dir().join(format!("rust_calckernel_build_f64_edge_c_runtime_{unique}"));
    fs::create_dir_all(&dir).expect("create temp dir");
    let runner = dir.join("run_f64_edge_c_dynamic.py");
    fs::write(&runner, official_c_dynamic_library_runner())
        .expect("write f64 edge C dynamic library runner");
    let source = typescript_root().join("tests/fixtures/f64_edges.ck");
    let ts_base = dir.join("ts_c_f64_edges");
    let rust_base = dir.join("rust_c_f64_edges");
    let ts_args = vec![
        os("build"),
        os("--out"),
        ts_base.clone().into_os_string(),
        os("--overflow"),
        os("unchecked"),
        os("-O3"),
        source.clone().into_os_string(),
    ];
    let rust_args = vec![
        os("build"),
        os("--out"),
        rust_base.clone().into_os_string(),
        os("--overflow"),
        os("unchecked"),
        os("-O3"),
        source.clone().into_os_string(),
    ];

    let ts_build = run_typescript_cli(&ts_cli, &ts_args);
    assert_eq!(
        ts_build.status_code,
        Some(0),
        "c-f64-edges TS stderr: {}",
        ts_build.stderr
    );
    let rust_build = run_rust_cli(&rust_args);
    assert_eq!(
        rust_build.status_code, ts_build.status_code,
        "c-f64-edges build status"
    );

    let ts_run = run_python_official_c_dynamic_library(
        &runner,
        "c-f64-edges",
        &shared_library_path(&ts_base),
    );
    let rust_run = run_python_official_c_dynamic_library(
        &runner,
        "c-f64-edges",
        &shared_library_path(&rust_base),
    );

    assert_eq!(
        ts_run.status_code,
        Some(0),
        "c-f64-edges TS runtime stderr: {}",
        ts_run.stderr
    );
    assert_eq!(
        rust_run.status_code,
        Some(0),
        "c-f64-edges Rust runtime stderr: {}",
        rust_run.stderr
    );
    assert_eq!(
        rust_run.status_code, ts_run.status_code,
        "c-f64-edges runtime status"
    );
    assert_eq!(rust_run.stdout, ts_run.stdout, "c-f64-edges runtime stdout");
    assert_eq!(rust_run.stderr, ts_run.stderr, "c-f64-edges runtime stderr");
}

#[test]
fn cli_should_print_mir_optimization_debug_output_to_stderr() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("rust_calckernel_cli_debug_{unique}"));
    fs::create_dir_all(&dir).expect("create temp dir");
    let source = dir.join("sample.ck");
    fs::write(
        &source,
        "export fn add_i64(a: i64, b: i64) -> i64 { return a + b; }",
    )
    .expect("write source");

    let output = Command::new(env!("CARGO_BIN_EXE_ckc"))
        .arg("emit-mir")
        .arg("-O3")
        .arg("--print-pass-pipeline")
        .arg("--print-mir-before-opt")
        .arg("--print-mir-after-opt")
        .arg(&source)
        .output()
        .expect("run emit-mir debug");
    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stdout.contains("export fn add_i64"));
    assert!(stderr.contains("MIR pass pipeline: O3:"));
    assert!(stderr.contains("MIR before optimization:"));
    assert!(stderr.contains("MIR after optimization:"));
    assert!(stderr.contains("%t0: i64 = add a, b"));
}

#[test]
fn cli_should_print_c_emission_pass_pipeline_to_stderr() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("rust_calckernel_cli_c_debug_{unique}"));
    fs::create_dir_all(&dir).expect("create temp dir");
    let source = dir.join("sample.ck");
    fs::write(
        &source,
        "export fn add_i64(a: i64, b: i64) -> i64 { return a + b; }",
    )
    .expect("write source");
    let c_path = dir.join("sample.c");

    let output = Command::new(env!("CARGO_BIN_EXE_ckc"))
        .arg("emit-c")
        .arg("-O3")
        .arg("--print-pass-pipeline")
        .arg("--out")
        .arg(&c_path)
        .arg(&source)
        .output()
        .expect("run emit-c debug");
    assert!(
        output.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stdout.contains("OK: emitted C with overflow=unchecked"));
    assert!(stderr.starts_with("MIR pass pipeline: O3:"));
    assert!(stderr.contains("loop-invariant-code-motion"));
}

#[test]
fn cli_should_match_typescript_oracle_for_public_error_cases() {
    let Some(ts_cli) = typescript_cli() else {
        return;
    };
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("rust_calckernel_cli_oracle_{unique}"));
    fs::create_dir_all(&dir).expect("create temp dir");
    let source = dir.join("sample.ck");
    fs::write(
        &source,
        "export fn add_i64(a: i64, b: i64) -> i64 { return a + b; }",
    )
    .expect("write source");
    let legacy_source = dir.join("sample.ik");

    let cases = vec![
        vec![
            os("emit-wat"),
            os("--overflow"),
            os("checked"),
            source.clone().into_os_string(),
        ],
        vec![
            os("emit-wasm"),
            os("--overflow"),
            os("checked"),
            os("--out"),
            dir.join("sample.wasm").into_os_string(),
            source.clone().into_os_string(),
        ],
        vec![
            os("emit-llvm"),
            os("--overflow"),
            os("checked"),
            source.clone().into_os_string(),
        ],
        vec![os("emit-mir"), os("-O9"), source.clone().into_os_string()],
        vec![
            os("emit-c"),
            os("--overflow"),
            os("strict"),
            os("--out"),
            dir.join("sample.c").into_os_string(),
            source.into_os_string(),
        ],
        vec![os("check"), legacy_source.into_os_string()],
    ];

    for args in cases {
        let ts = run_typescript_cli(&ts_cli, &args);
        let rust = run_rust_cli(&args);
        assert_eq!(rust.status_code, ts.status_code, "args: {args:?}");
        assert_eq!(rust.stdout, ts.stdout, "args: {args:?}");
        assert_eq!(rust.stderr, ts.stderr, "args: {args:?}");
    }
}

#[test]
fn cli_should_match_typescript_oracle_for_help_check_and_mir_debug() {
    let Some(ts_cli) = typescript_cli() else {
        return;
    };
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("rust_calckernel_cli_success_oracle_{unique}"));
    fs::create_dir_all(&dir).expect("create temp dir");
    let source = dir.join("sample.ck");
    fs::write(
        &source,
        "export fn add_i64(a: i64, b: i64) -> i64 { return a + b; }",
    )
    .expect("write source");

    let cases = vec![
        vec![os("--help")],
        vec![os("check"), source.clone().into_os_string()],
        vec![
            os("emit-mir"),
            os("-O3"),
            os("--print-pass-pipeline"),
            os("--print-mir-before-opt"),
            os("--print-mir-after-opt"),
            source.into_os_string(),
        ],
    ];

    for args in cases {
        let ts = run_typescript_cli(&ts_cli, &args);
        let rust = run_rust_cli(&args);
        assert_eq!(rust.status_code, ts.status_code, "args: {args:?}");
        assert_eq!(legacy_cli_text(&rust.stdout), ts.stdout, "args: {args:?}");
        assert_eq!(legacy_cli_text(&rust.stderr), ts.stderr, "args: {args:?}");
    }
}

#[test]
fn cli_should_match_typescript_oracle_for_successful_emit_commands() {
    let Some(ts_cli) = typescript_cli() else {
        return;
    };
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("rust_calckernel_cli_emit_oracle_{unique}"));
    fs::create_dir_all(&dir).expect("create temp dir");
    let source = dir.join("sample.ck");
    fs::write(
        &source,
        "export fn add_i64(a: i64, b: i64) -> i64 { return a + b; }",
    )
    .expect("write source");

    let cases = vec![
        vec![
            os("emit-c"),
            os("--out"),
            dir.join("sample.c").into_os_string(),
            source.clone().into_os_string(),
        ],
        vec![
            os("emit-c"),
            os("--overflow"),
            os("checked"),
            os("--out"),
            dir.join("sample_checked.c").into_os_string(),
            source.clone().into_os_string(),
        ],
        vec![
            os("emit-wat"),
            os("--out"),
            dir.join("sample.wat").into_os_string(),
            source.clone().into_os_string(),
        ],
        vec![os("emit-wat"), source.clone().into_os_string()],
        vec![
            os("emit-wasm"),
            os("--out"),
            dir.join("sample.wasm").into_os_string(),
            source.clone().into_os_string(),
        ],
        vec![
            os("emit-llvm"),
            os("--target"),
            os("ck-test-target"),
            os("--out"),
            dir.join("sample.ll").into_os_string(),
            source.clone().into_os_string(),
        ],
        vec![
            os("emit-llvm"),
            os("--target"),
            os("ck-test-target"),
            source.into_os_string(),
        ],
    ];

    for args in cases {
        let ts = run_typescript_cli(&ts_cli, &args);
        let rust = run_rust_cli(&args);
        assert_eq!(rust.status_code, ts.status_code, "args: {args:?}");
        assert_eq!(legacy_cli_text(&rust.stdout), ts.stdout, "args: {args:?}");
        assert_eq!(rust.stderr, ts.stderr, "args: {args:?}");
    }
}

#[test]
fn cli_should_match_typescript_oracle_for_diagnostics() {
    let Some(ts_cli) = typescript_cli() else {
        return;
    };
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("rust_calckernel_cli_diag_oracle_{unique}"));
    fs::create_dir_all(&dir).expect("create temp dir");

    let cases = [
        ("lex.ck", "export fn bad() -> i32 {\n  return @;\n}\n"),
        (
            "parse.ck",
            "export fn bad() -> i32 {\n  let x: i32 = 1\n  return x;\n}\n",
        ),
        (
            "unknown_variable.ck",
            "export fn bad() -> i32 {\n  return missing;\n}\n",
        ),
        (
            "return_mismatch.ck",
            "export fn bad() -> i32 {\n  return true;\n}\n",
        ),
        (
            "non_bmp_unknown_character.ck",
            "export fn bad() -> i32 { return 🙂; }\n",
        ),
        (
            "multiline_unicode_marker_width.ck",
            "export fn bad() -> i32 { // 中文🙂\n  let x: i32 = 1;\n}\n",
        ),
    ];

    for (file_name, source_text) in cases {
        let source = dir.join(file_name);
        fs::write(&source, source_text).expect("write diagnostic source");
        let args = vec![os("check"), source.into_os_string()];
        let ts = run_typescript_cli(&ts_cli, &args);
        let rust = run_rust_cli(&args);
        assert_eq!(rust.status_code, ts.status_code, "file: {file_name}");
        assert_eq!(rust.stdout, ts.stdout, "file: {file_name}");
        assert_eq!(rust.stderr, ts.stderr, "file: {file_name}");
    }
}

#[test]
fn cli_should_match_typescript_oracle_for_usage_errors() {
    let Some(ts_cli) = typescript_cli() else {
        return;
    };
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("rust_calckernel_cli_usage_oracle_{unique}"));
    fs::create_dir_all(&dir).expect("create temp dir");
    let source = dir.join("sample.ck");
    fs::write(
        &source,
        "export fn add_i64(a: i64, b: i64) -> i64 { return a + b; }",
    )
    .expect("write source");

    let cases = vec![
        vec![],
        vec![os("check")],
        vec![
            os("check"),
            source.clone().into_os_string(),
            source.clone().into_os_string(),
        ],
        vec![os("emit-c"), source.clone().into_os_string()],
        vec![os("emit-wasm"), source.clone().into_os_string()],
        vec![os("emit-c"), os("--out")],
        vec![os("emit-c"), os("-o")],
        vec![os("emit-c"), os("--overflow")],
    ];

    for args in cases {
        let ts = run_typescript_cli(&ts_cli, &args);
        let rust = run_rust_cli(&args);
        assert_eq!(rust.status_code, ts.status_code, "args: {args:?}");
        assert_eq!(legacy_cli_text(&rust.stdout), ts.stdout, "args: {args:?}");
        assert_eq!(legacy_cli_text(&rust.stderr), ts.stderr, "args: {args:?}");
    }
}

#[test]
fn cli_should_match_typescript_oracle_for_ignored_unknown_long_flags() {
    let Some(ts_cli) = typescript_cli() else {
        return;
    };
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    let dir =
        std::env::temp_dir().join(format!("rust_calckernel_cli_unknown_flag_oracle_{unique}"));
    fs::create_dir_all(&dir).expect("create temp dir");
    let source = dir.join("sample.ck");
    fs::write(
        &source,
        "export fn add_i64(a: i64, b: i64) -> i64 { return a + b; }",
    )
    .expect("write source");

    let cases = vec![
        vec![
            os("check"),
            os("--unused"),
            os("value"),
            source.clone().into_os_string(),
        ],
        vec![
            os("emit-c"),
            os("--unused"),
            os("value"),
            os("--out"),
            dir.join("sample.c").into_os_string(),
            source.into_os_string(),
        ],
    ];

    for args in cases {
        let ts = run_typescript_cli(&ts_cli, &args);
        let rust = run_rust_cli(&args);
        assert_eq!(rust.status_code, ts.status_code, "args: {args:?}");
        assert_eq!(legacy_cli_text(&rust.stdout), ts.stdout, "args: {args:?}");
        assert_eq!(legacy_cli_text(&rust.stderr), ts.stderr, "args: {args:?}");
    }
}

#[test]
fn cli_should_match_typescript_oracle_for_unknown_short_flags_as_positionals() {
    let Some(ts_cli) = typescript_cli() else {
        return;
    };
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    let dir =
        std::env::temp_dir().join(format!("rust_calckernel_cli_unknown_short_oracle_{unique}"));
    fs::create_dir_all(&dir).expect("create temp dir");
    let source = dir.join("sample.ck");
    fs::write(
        &source,
        "export fn add_i64(a: i64, b: i64) -> i64 { return a + b; }",
    )
    .expect("write source");

    let args = vec![os("check"), os("-x"), source.into_os_string()];
    let ts = run_typescript_cli(&ts_cli, &args);
    let rust = run_rust_cli(&args);
    assert_eq!(rust.status_code, ts.status_code, "args: {args:?}");
    assert_eq!(legacy_cli_text(&rust.stdout), ts.stdout, "args: {args:?}");
    assert_eq!(legacy_cli_text(&rust.stderr), ts.stderr, "args: {args:?}");
}

#[test]
fn cli_should_defer_semantic_flag_validation_until_command_uses_flag() {
    let Some(ts_cli) = typescript_cli() else {
        return;
    };
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "rust_calckernel_deferred_flag_validation_oracle_{unique}"
    ));
    fs::create_dir_all(&dir).expect("create temp dir");
    let source = dir.join("sample.ck");
    fs::write(
        &source,
        "export fn add_i64(a: i64, b: i64) -> i64 { return a + b; }",
    )
    .expect("write source");

    let cases = vec![
        vec![
            os("check"),
            os("--overflow"),
            os("bogus"),
            source.clone().into_os_string(),
        ],
        vec![os("check"), os("-O4"), source.clone().into_os_string()],
        vec![
            os("check"),
            os("--opt-level"),
            os("-O2"),
            source.clone().into_os_string(),
        ],
        vec![
            os("emit-mir"),
            os("--overflow"),
            os("bogus"),
            source.clone().into_os_string(),
        ],
        vec![
            os("emit-mir"),
            os("--out"),
            dir.join("sample.mir").into_os_string(),
            os("--overflow"),
            os("bogus"),
            source.into_os_string(),
        ],
    ];

    for args in cases {
        let ts = run_typescript_cli(&ts_cli, &args);
        let rust = run_rust_cli(&args);
        assert_eq!(rust.status_code, ts.status_code, "args: {args:?}");
        assert_eq!(rust.stdout, ts.stdout, "args: {args:?}");
        assert_eq!(rust.stderr, ts.stderr, "args: {args:?}");
    }
}

#[test]
fn cli_should_match_typescript_oracle_for_argument_error_precedence() {
    let Some(ts_cli) = typescript_cli() else {
        return;
    };
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    let dir =
        std::env::temp_dir().join(format!("rust_calckernel_error_precedence_oracle_{unique}"));
    fs::create_dir_all(&dir).expect("create temp dir");
    let source = dir.join("sample.ck");
    fs::write(
        &source,
        "export fn add_i64(a: i64, b: i64) -> i64 { return a + b; }",
    )
    .expect("write source");

    let cases = vec![
        vec![os("emit-c"), os("--opt-level"), os("4")],
        vec![
            os("build"),
            os("--overflow"),
            os("bogus"),
            source.clone().into_os_string(),
        ],
        vec![os("emit-wat"), os("--overflow"), os("checked")],
        vec![
            os("emit-wasm"),
            os("--overflow"),
            os("checked"),
            source.clone().into_os_string(),
        ],
        vec![os("emit-llvm"), os("--overflow"), os("checked")],
        vec![
            os("build-llvm"),
            os("--overflow"),
            os("checked"),
            source.into_os_string(),
        ],
    ];

    for args in cases {
        let ts = run_typescript_cli(&ts_cli, &args);
        let rust = run_rust_cli(&args);
        assert_eq!(rust.status_code, ts.status_code, "args: {args:?}");
        assert_eq!(legacy_cli_text(&rust.stdout), ts.stdout, "args: {args:?}");
        assert_eq!(legacy_cli_text(&rust.stderr), ts.stderr, "args: {args:?}");
    }
}

#[test]
fn cli_should_match_typescript_oracle_for_unknown_command_precedence() {
    let Some(ts_cli) = typescript_cli() else {
        return;
    };
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("rust_calckernel_unknown_command_oracle_{unique}"));
    fs::create_dir_all(&dir).expect("create temp dir");
    let source = dir.join("sample.ck");
    fs::write(
        &source,
        "export fn add_i64(a: i64, b: i64) -> i64 { return a + b; }",
    )
    .expect("write source");

    let cases = vec![
        vec![os("unknown"), os("--out"), os("--other"), os("--other")],
        vec![
            os("unknown"),
            source.clone().into_os_string(),
            os("-o"),
            os("--header"),
            os("ck-test-target"),
        ],
        vec![os("checked"), os("value"), os("--overflow")],
        vec![os("4"), os("--target")],
    ];

    for args in cases {
        let ts = run_typescript_cli(&ts_cli, &args);
        let rust = run_rust_cli(&args);
        assert_eq!(rust.status_code, ts.status_code, "args: {args:?}");
        assert_eq!(legacy_cli_text(&rust.stdout), ts.stdout, "args: {args:?}");
        assert_eq!(legacy_cli_text(&rust.stderr), ts.stderr, "args: {args:?}");
    }
}

#[test]
fn cli_should_match_typescript_oracle_for_missing_input_file_errors() {
    let Some(ts_cli) = typescript_cli() else {
        return;
    };
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("rust_calckernel_missing_file_oracle_{unique}"));
    fs::create_dir_all(&dir).expect("create temp dir");
    let missing = dir.join("missing.ck");

    let cases = vec![
        vec![os("check"), missing.clone().into_os_string()],
        vec![
            os("emit-mir"),
            os("--header"),
            os("value"),
            missing.into_os_string(),
            os("--other"),
            dir.join("api.h").into_os_string(),
        ],
    ];

    for args in cases {
        let ts = run_typescript_cli(&ts_cli, &args);
        let rust = run_rust_cli(&args);
        assert_eq!(rust.status_code, ts.status_code, "args: {args:?}");
        assert_eq!(rust.stdout, ts.stdout, "args: {args:?}");
        assert_eq!(rust.stderr, ts.stderr, "args: {args:?}");
    }
}

#[test]
fn cli_should_match_typescript_oracle_for_input_file_read_edge_cases() {
    let Some(ts_cli) = typescript_cli() else {
        return;
    };
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("rust_calckernel_file_read_oracle_{unique}"));
    fs::create_dir_all(&dir).expect("create temp dir");
    let directory_input = dir.join("directory.ck");
    fs::create_dir_all(&directory_input).expect("create input directory");
    let invalid_utf8 = dir.join("invalid_utf8.ck");
    fs::write(&invalid_utf8, [0xff, b'\n']).expect("write invalid utf8 source");

    let cases = vec![
        vec![os("check"), directory_input.into_os_string()],
        vec![os("check"), invalid_utf8.into_os_string()],
    ];

    for args in cases {
        let ts = run_typescript_cli(&ts_cli, &args);
        let rust = run_rust_cli(&args);
        assert_eq!(rust.status_code, ts.status_code, "args: {args:?}");
        assert_eq!(rust.stdout, ts.stdout, "args: {args:?}");
        assert_eq!(rust.stderr, ts.stderr, "args: {args:?}");
    }
}

#[test]
fn cli_should_match_typescript_oracle_for_direct_output_write_errors() {
    let Some(ts_cli) = typescript_cli() else {
        return;
    };
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("rust_calckernel_direct_write_oracle_{unique}"));
    fs::create_dir_all(&dir).expect("create temp dir");
    let source = dir.join("sample.ck");
    fs::write(
        &source,
        "export fn add_i64(a: i64, b: i64) -> i64 { return a + b; }",
    )
    .expect("write source");
    let output_directory = dir.join("out.mir");
    fs::create_dir_all(&output_directory).expect("create output directory");

    let args = vec![
        os("emit-mir"),
        source.into_os_string(),
        os("--out"),
        output_directory.into_os_string(),
    ];
    let ts = run_typescript_cli(&ts_cli, &args);
    let rust = run_rust_cli(&args);
    assert_eq!(rust.status_code, ts.status_code, "args: {args:?}");
    assert_eq!(rust.stdout, ts.stdout, "args: {args:?}");
    assert_eq!(rust.stderr, ts.stderr, "args: {args:?}");
}

#[test]
fn cli_should_match_typescript_oracle_for_atomic_output_write_errors() {
    let Some(ts_cli) = typescript_cli() else {
        return;
    };
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("rust_calckernel_atomic_write_oracle_{unique}"));
    fs::create_dir_all(&dir).expect("create temp dir");
    let source = dir.join("sample.ck");
    fs::write(
        &source,
        "export fn add_i64(a: i64, b: i64) -> i64 { return a + b; }",
    )
    .expect("write source");
    let output_directory = dir.join("out.wat");
    fs::create_dir_all(&output_directory).expect("create output directory");

    let args = vec![
        os("emit-wat"),
        source.into_os_string(),
        os("--out"),
        output_directory.into_os_string(),
    ];
    let ts = run_typescript_cli(&ts_cli, &args);
    let rust = run_rust_cli(&args);
    assert_eq!(rust.status_code, ts.status_code, "args: {args:?}");
    assert_eq!(rust.stdout, ts.stdout, "args: {args:?}");
    assert_eq!(
        normalize_atomic_temp_paths(&rust.stderr),
        normalize_atomic_temp_paths(&ts.stderr),
        "args: {args:?}"
    );
}

#[test]
fn cli_should_match_typescript_oracle_for_output_parent_directory_creation_errors() {
    let Some(ts_cli) = typescript_cli() else {
        return;
    };
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("rust_calckernel_parent_dir_oracle_{unique}"));
    fs::create_dir_all(&dir).expect("create temp dir");
    let source = dir.join("sample.ck");
    fs::write(
        &source,
        "export fn add_i64(a: i64, b: i64) -> i64 { return a + b; }",
    )
    .expect("write source");
    let parent_file = dir.join("parent");
    fs::write(&parent_file, "not a directory").expect("write parent file");

    let cases = vec![
        vec![
            os("emit-mir"),
            source.clone().into_os_string(),
            os("--out"),
            parent_file.join("out.mir").into_os_string(),
        ],
        vec![
            os("emit-wat"),
            source.clone().into_os_string(),
            os("--out"),
            parent_file.join("out.wat").into_os_string(),
        ],
        vec![
            os("emit-c"),
            source.clone().into_os_string(),
            os("--out"),
            dir.join("ok.c").into_os_string(),
            os("--header"),
            parent_file.join("out.h").into_os_string(),
        ],
        vec![
            os("emit-mir"),
            source.clone().into_os_string(),
            os("--out"),
            parent_file.join("child").join("out.mir").into_os_string(),
        ],
        vec![
            os("emit-wat"),
            source.clone().into_os_string(),
            os("--out"),
            parent_file.join("child").join("out.wat").into_os_string(),
        ],
        vec![
            os("emit-c"),
            source.clone().into_os_string(),
            os("--out"),
            dir.join("ok_nested.c").into_os_string(),
            os("--header"),
            parent_file.join("child").join("out.h").into_os_string(),
        ],
    ];

    for args in cases {
        let ts = run_typescript_cli(&ts_cli, &args);
        let rust = run_rust_cli(&args);
        assert_eq!(rust.status_code, ts.status_code, "args: {args:?}");
        assert_eq!(rust.stdout, ts.stdout, "args: {args:?}");
        assert_eq!(rust.stderr, ts.stderr, "args: {args:?}");
    }
}

#[test]
fn cli_should_default_bounds_to_unchecked_for_all_emitters() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("rust_calckernel_bounds_default_{unique}"));
    fs::create_dir_all(&dir).expect("create temp dir");
    let source = dir.join("slice.ck");
    fs::write(
        &source,
        "export fn first(items: slice<i32>) -> i32 { return items[0]; }",
    )
    .expect("write source");

    let cases = [
        vec![
            os("emit-c"),
            source.clone().into_os_string(),
            os("--out"),
            dir.join("out.c").into_os_string(),
        ],
        vec![os("emit-wat"), source.clone().into_os_string()],
        vec![
            os("emit-wasm"),
            source.clone().into_os_string(),
            os("--out"),
            dir.join("out.wasm").into_os_string(),
        ],
        vec![os("emit-llvm"), source.clone().into_os_string()],
    ];
    for args in cases {
        let output = run_rust_cli(&args);
        assert_eq!(
            output.status_code,
            Some(0),
            "args={args:?}: {}",
            output.stderr
        );
        assert!(!output.stderr.contains("bounds"), "args={args:?}");
    }

    let c = run_rust_cli(&[
        os("emit-c"),
        source.into_os_string(),
        os("--out"),
        dir.join("reported.c").into_os_string(),
    ]);
    assert!(c.stdout.contains("bounds=unchecked"), "{}", c.stdout);
    let help = run_rust_cli(&[os("--help")]);
    assert!(help.stdout.contains("--bounds <unchecked|checked>"));
    assert!(help.stdout.contains("Default: unchecked"));
}

#[test]
fn cli_should_accept_all_four_c_overflow_bounds_combinations() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("rust_calckernel_bounds_c_matrix_{unique}"));
    fs::create_dir_all(&dir).expect("create temp dir");
    let source = dir.join("slice.ck");
    fs::write(
        &source,
        "export fn first(items: slice<i32>) -> i32 { return items[0] + 1; }",
    )
    .expect("write source");

    for overflow in ["unchecked", "checked"] {
        for bounds in ["unchecked", "checked"] {
            let output_path = dir.join(format!("{overflow}_{bounds}.c"));
            let emit = run_rust_cli(&[
                os("emit-c"),
                source.clone().into_os_string(),
                os("--overflow"),
                os(overflow),
                os("--bounds"),
                os(bounds),
                os("--out"),
                output_path.into_os_string(),
            ]);
            assert_eq!(
                emit.status_code,
                Some(0),
                "{overflow}/{bounds}: {}",
                emit.stderr
            );
            assert!(emit.stdout.contains(&format!("overflow={overflow}")));
            assert!(emit.stdout.contains(&format!("bounds={bounds}")));

            let build = run_rust_cli(&[
                os("build"),
                source.clone().into_os_string(),
                os("--overflow"),
                os(overflow),
                os("--bounds"),
                os(bounds),
                os("--out"),
                dir.join(format!("lib_{overflow}_{bounds}"))
                    .into_os_string(),
            ]);
            assert_eq!(
                build.status_code,
                Some(0),
                "build {overflow}/{bounds}: {}",
                build.stderr
            );
            assert!(build.stdout.contains(&format!("overflow={overflow}")));
            assert!(build.stdout.contains(&format!("bounds={bounds}")));
        }
    }
}

#[test]
fn cli_should_reject_invalid_bounds_values_on_commands_that_use_them() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("rust_calckernel_bounds_invalid_{unique}"));
    fs::create_dir_all(&dir).expect("create temp dir");
    let source = dir.join("sample.ck");
    fs::write(&source, "export fn value() -> i32 { return 1; }").expect("write source");
    let cases = [
        vec![
            os("emit-c"),
            source.clone().into_os_string(),
            os("--out"),
            dir.join("out.c").into_os_string(),
            os("--bounds"),
            os("invalid"),
        ],
        vec![
            os("build"),
            source.clone().into_os_string(),
            os("--out"),
            dir.join("lib").into_os_string(),
            os("--bounds"),
            os("invalid"),
        ],
        vec![
            os("emit-wat"),
            source.clone().into_os_string(),
            os("--bounds"),
            os("invalid"),
        ],
        vec![
            os("emit-wasm"),
            source.clone().into_os_string(),
            os("--out"),
            dir.join("out.wasm").into_os_string(),
            os("--bounds"),
            os("invalid"),
        ],
        vec![
            os("emit-llvm"),
            source.clone().into_os_string(),
            os("--bounds"),
            os("invalid"),
        ],
        vec![
            os("build-llvm"),
            source.into_os_string(),
            os("--out"),
            dir.join("llvm").into_os_string(),
            os("--bounds"),
            os("invalid"),
        ],
    ];
    for args in cases {
        let output = run_rust_cli(&args);
        assert_eq!(output.status_code, Some(1), "args={args:?}");
        assert_eq!(
            output.stderr,
            "Invalid value for --bounds: invalid. Expected 'unchecked' or 'checked'.\n",
            "args={args:?}"
        );
    }
}

#[test]
fn cli_should_reject_checked_bounds_for_wasm_and_llvm_with_stable_help() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("rust_calckernel_bounds_unsupported_{unique}"));
    fs::create_dir_all(&dir).expect("create temp dir");
    let source = dir.join("sample.ck");
    fs::write(&source, "export fn value() -> i32 { return 1; }").expect("write source");

    for (command, backend, out) in [
        ("emit-wat", "WASM", None),
        ("emit-wasm", "WASM", Some(dir.join("out.wasm"))),
        ("emit-llvm", "LLVM", None),
        ("build-llvm", "LLVM", Some(dir.join("llvm"))),
    ] {
        let mut args = vec![
            os(command),
            source.clone().into_os_string(),
            os("--bounds"),
            os("checked"),
        ];
        if let Some(out) = out {
            args.push(os("--out"));
            args.push(out.into_os_string());
        }
        let output = run_rust_cli(&args);
        assert_eq!(output.status_code, Some(1), "{command}");
        assert!(
            output.stderr.contains(&format!(
                "{backend} backend does not support --bounds checked yet."
            )),
            "{}",
            output.stderr
        );
        assert!(output.stderr.contains("help:"), "{}", output.stderr);
        assert!(
            output.stderr.contains("--bounds unchecked"),
            "{}",
            output.stderr
        );
    }
}

#[test]
fn cli_should_report_checked_overflow_before_checked_bounds_for_wasm_and_llvm() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("rust_calckernel_bounds_precedence_{unique}"));
    fs::create_dir_all(&dir).expect("create temp dir");
    let source = dir.join("sample.ck");
    fs::write(&source, "export fn value() -> i32 { return 1; }").expect("write source");

    for (command, backend) in [("emit-wat", "WASM"), ("emit-llvm", "LLVM")] {
        for flags in [
            ["--overflow", "checked", "--bounds", "checked"],
            ["--bounds", "checked", "--overflow", "checked"],
        ] {
            let mut args = vec![os(command), source.clone().into_os_string()];
            args.extend(flags.into_iter().map(os));
            let output = run_rust_cli(&args);
            assert_eq!(output.status_code, Some(1), "{command} {flags:?}");
            assert!(
                output.stderr.contains(&format!(
                    "{backend} backend does not support --overflow checked yet."
                )),
                "{}",
                output.stderr
            );
            assert!(!output.stderr.contains("--bounds checked yet"));
        }
    }
}

#[test]
fn cli_should_keep_check_and_emit_mir_source_semantics_mode_independent() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("rust_calckernel_bounds_semantic_{unique}"));
    fs::create_dir_all(&dir).expect("create temp dir");
    let source = dir.join("sample.ck");
    fs::write(&source, "export fn value() -> i32 { return 1; }").expect("write source");

    for command in ["check", "emit-mir"] {
        let output = run_rust_cli(&[
            os(command),
            source.clone().into_os_string(),
            os("--overflow"),
            os("not-a-mode"),
            os("--bounds"),
            os("also-not-a-mode"),
        ]);
        assert_eq!(output.status_code, Some(0), "{command}: {}", output.stderr);
    }
}

#[derive(Debug)]
struct CapturedOutput {
    status_code: Option<i32>,
    stdout: String,
    stderr: String,
}

fn os(value: &str) -> OsString {
    OsString::from(value)
}

fn legacy_cli_text(text: &str) -> String {
    text.replace(", bounds=unchecked", "")
        .replace(", bounds=checked", "")
        .replace(" [--bounds <unchecked|checked>]", "")
        .replace(" [--bounds unchecked]", "")
        .replace(
            "  --bounds <unchecked|checked>      Slice bounds mode. Default: unchecked; checked is C-only.\n",
            "",
        )
}

fn normalize_atomic_temp_paths(text: &str) -> String {
    let mut normalized = String::new();
    let mut rest = text;
    while let Some(index) = rest.find(".tmp-") {
        let (before, after) = rest.split_at(index);
        normalized.push_str(before);
        normalized.push_str(".tmp-<id>");
        let Some(end_quote) = after.find('\'') else {
            return normalized;
        };
        rest = &after[end_quote..];
    }
    normalized.push_str(rest);
    normalized
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

fn run_rust_cli(args: &[OsString]) -> CapturedOutput {
    let output = Command::new(env!("CARGO_BIN_EXE_ckc"))
        .args(args)
        .output()
        .expect("run Rust ckc");
    capture(output)
}

fn run_typescript_cli(ts_cli: &PathBuf, args: &[OsString]) -> CapturedOutput {
    let output = Command::new("node")
        .arg(ts_cli)
        .args(args)
        .output()
        .expect("run TypeScript ckc");
    capture(output)
}

#[test]
fn cli_should_build_void_buffer_mutation_fixture() {
    if !clang_available() {
        return;
    }
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("rust_calckernel_cli_void_{unique}"));
    fs::create_dir_all(&dir).expect("create temp dir");
    let source = dir.join("void.ck");
    fs::write(
        &source,
        "export fn mutate(out: ptr<i32>) -> void { out[0] = 42; }",
    )
    .expect("write source");
    let output_base = dir.join("libvoid");
    let build = Command::new(env!("CARGO_BIN_EXE_ckc"))
        .arg("build")
        .arg("--out")
        .arg(&output_base)
        .arg(&source)
        .output()
        .expect("run ckc build");
    assert!(
        build.status.success(),
        "ckc stderr:\n{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let header = fs::read_to_string(output_base.with_extension("h")).expect("read header");
    assert!(
        header.contains("CK_API void mutate(int32_t* out);"),
        "{header}"
    );

    let harness = dir.join("harness.c");
    fs::write(
        &harness,
        r#"
#include "libvoid.h"
int main(void) {
  int32_t value = 0;
  mutate(&value);
  return value == 42 ? 0 : 1;
}
"#,
    )
    .expect("write harness");
    let binary = dir.join("harness");
    let compile = Command::new("clang")
        .arg("-std=c11")
        .arg("-Wall")
        .arg("-Wextra")
        .arg("-Werror")
        .arg(&harness)
        .arg(output_base.with_extension("c"))
        .arg("-I")
        .arg(&dir)
        .arg("-o")
        .arg(&binary)
        .output()
        .expect("compile harness");
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

fn clang_available() -> bool {
    Command::new("clang")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
}

fn python3_available() -> bool {
    Command::new("python3")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
}

fn run_python_pricing_dynamic_library(
    runner: &std::path::Path,
    mode: &str,
    library_path: &std::path::Path,
) -> CapturedOutput {
    let output = Command::new("python3")
        .arg(runner)
        .arg(mode)
        .arg(library_path)
        .output()
        .expect("run pricing dynamic library runner");
    capture(output)
}

fn run_python_official_c_dynamic_library(
    runner: &std::path::Path,
    case_name: &str,
    library_path: &std::path::Path,
) -> CapturedOutput {
    let output = Command::new("python3")
        .arg(runner)
        .arg(case_name)
        .arg(library_path)
        .output()
        .expect("run official C dynamic library runner");
    capture(output)
}

fn capture(output: std::process::Output) -> CapturedOutput {
    CapturedOutput {
        status_code: output.status.code(),
        stdout: String::from_utf8(output.stdout).expect("stdout should be utf8"),
        stderr: String::from_utf8(output.stderr).expect("stderr should be utf8"),
    }
}

fn shared_library_path(base: &std::path::Path) -> std::path::PathBuf {
    if cfg!(target_os = "macos") {
        base.with_extension("dylib")
    } else if cfg!(target_os = "windows") {
        base.with_extension("dll")
    } else {
        base.with_extension("so")
    }
}

fn pricing_dynamic_library_runner() -> &'static str {
    r#"
from __future__ import annotations

import ctypes
import sys


CK_OK = 0
CK_ERR_OVERFLOW = 1


class Item(ctypes.Structure):
    _fields_ = [
        ("price", ctypes.c_int64),
        ("qty", ctypes.c_int64),
        ("discount", ctypes.c_int64),
        ("tax_rate_ppm", ctypes.c_int64),
    ]


def pricing_items():
    return (Item * 3)(
        Item(price=10000, qty=2, discount=1000, tax_rate_ppm=82500),
        Item(price=2500, qty=4, discount=0, tax_rate_ppm=100000),
        Item(price=1200, qty=5, discount=500, tax_rate_ppm=100000),
    )


def run_unchecked(library_path: str) -> str:
    lib = ctypes.CDLL(library_path)
    lib.calc_items.argtypes = [
        ctypes.POINTER(Item),
        ctypes.c_int32,
        ctypes.POINTER(ctypes.c_int64),
    ]
    lib.calc_items.restype = ctypes.c_int32

    items = pricing_items()
    out = (ctypes.c_int64 * len(items))(0, 0, 0)
    status = lib.calc_items(items, ctypes.c_int32(len(items)), out)
    expected = [20567, 11000, 6050]
    actual = list(out)
    if status != 0 or actual != expected:
        raise AssertionError(f"unchecked mismatch status={status} actual={actual} expected={expected}")
    return f"pricing:status={status};out={','.join(str(value) for value in actual)}"


def run_checked(library_path: str) -> str:
    lib = ctypes.CDLL(library_path)
    lib.calc_items.argtypes = [
        ctypes.POINTER(Item),
        ctypes.c_int32,
        ctypes.POINTER(ctypes.c_int64),
        ctypes.POINTER(ctypes.c_int32),
    ]
    lib.calc_items.restype = ctypes.c_int32

    items = pricing_items()
    out = (ctypes.c_int64 * len(items))(0, 0, 0)
    ck_return = ctypes.c_int32(-1)
    status = lib.calc_items(items, ctypes.c_int32(len(items)), out, ctypes.byref(ck_return))
    expected = [20567, 11000, 6050]
    actual = list(out)
    if status != CK_OK or ck_return.value != 0 or actual != expected:
        raise AssertionError(
            f"checked mismatch status={status} ck_return={ck_return.value} actual={actual} expected={expected}"
        )

    overflow_items = (Item * 1)(
        Item(price=ctypes.c_int64(9223372036854775807).value, qty=2, discount=0, tax_rate_ppm=0),
    )
    overflow_out = (ctypes.c_int64 * 1)(0)
    overflow_return = ctypes.c_int32(-1)
    overflow_status = lib.calc_items(
        overflow_items,
        ctypes.c_int32(len(overflow_items)),
        overflow_out,
        ctypes.byref(overflow_return),
    )
    if overflow_status != CK_ERR_OVERFLOW:
        raise AssertionError(f"expected CK_ERR_OVERFLOW, got CK_Status {overflow_status}")

    return (
        f"pricing-checked:status={status};ck_return={ck_return.value};"
        f"out={','.join(str(value) for value in actual)};overflow_status={overflow_status}"
    )


def main() -> None:
    mode, library_path = sys.argv[1:3]
    if mode == "unchecked":
        print(run_unchecked(library_path))
    elif mode == "checked":
        print(run_checked(library_path))
    else:
        raise RuntimeError(f"unknown mode: {mode}")


if __name__ == "__main__":
    main()
"#
}

fn official_c_dynamic_library_runner() -> &'static str {
    r#"
from __future__ import annotations

import ctypes
import math
import sys


CK_OK = 0
CK_ERR_OVERFLOW = 1
CK_ERR_DIV_BY_ZERO = 2


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


def checked_i64_call(fn, *args: int) -> tuple[int, int]:
    result = ctypes.c_int64(-1)
    status = fn(*args, ctypes.byref(result))
    return status, result.value


def checked_bool_call(fn, *args) -> tuple[int, bool]:
    result = ctypes.c_bool(False)
    status = fn(*args, ctypes.byref(result))
    return status, bool(result.value)


def close(actual: float, expected: float) -> bool:
    return math.isclose(actual, expected, rel_tol=0.0, abs_tol=0.0000001)


def format_float(value: float) -> str:
    return format(value, ".17g")


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


def ok(value: bool) -> str:
    return "ok" if value else "fail"


def run_c_scalar(library_path: str) -> str:
    lib = ctypes.CDLL(library_path)
    lib.add_i64.argtypes = [ctypes.c_int64, ctypes.c_int64]
    lib.add_i64.restype = ctypes.c_int64
    lib.max_i32.argtypes = [ctypes.c_int32, ctypes.c_int32]
    lib.max_i32.restype = ctypes.c_int32

    add = lib.add_i64(2, 3)
    high = lib.max_i32(10, 3)
    low = lib.max_i32(1, 3)
    if add != 5 or high != 10 or low != 3:
        raise AssertionError(f"c-scalar mismatch add={add} high={high} low={low}")
    return f"c-scalar:add={add};max={high},{low}"


def run_c_casts(library_path: str) -> str:
    lib = ctypes.CDLL(library_path)
    lib.avg_i32.argtypes = [ctypes.c_int32, ctypes.c_int32]
    lib.avg_i32.restype = ctypes.c_double
    lib.ratio_u32.argtypes = [ctypes.c_uint32, ctypes.c_uint32]
    lib.ratio_u32.restype = ctypes.c_double

    avg = lib.avg_i32(7, 2)
    ratio = lib.ratio_u32(9, 4)
    if not close(avg, 3.5) or not close(ratio, 2.25):
        raise AssertionError(f"c-casts mismatch avg={avg} ratio={ratio}")
    return f"c-casts:avg={avg};ratio={ratio}"


def run_c_f64_array(library_path: str) -> str:
    lib = ctypes.CDLL(library_path)
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
    if not close(checksum, expected_checksum) or any(
        not close(value, expected[index]) for index, value in enumerate(actual)
    ):
        raise AssertionError(f"c-f64-array mismatch checksum={checksum} actual={actual}")
    return (
        f"c-f64-array:checksum={format_float(checksum)};"
        f"out={','.join(format_float(value) for value in actual)}"
    )


def run_c_f64_axpy(library_path: str) -> str:
    lib = ctypes.CDLL(library_path)
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
    if not close(checksum, expected_checksum) or any(
        not close(value, expected[index]) for index, value in enumerate(actual)
    ):
        raise AssertionError(f"c-f64-axpy mismatch checksum={checksum} actual={actual}")
    return (
        f"c-f64-axpy:checksum={format_float(checksum)};"
        f"out={','.join(format_float(value) for value in actual)}"
    )


def run_c_f64_sum(library_path: str) -> str:
    lib = ctypes.CDLL(library_path)
    double_ptr = ctypes.POINTER(ctypes.c_double)
    lib.sum_f64.argtypes = [double_ptr, ctypes.c_int32]
    lib.sum_f64.restype = ctypes.c_double

    input_values = [1.25, -2.5, 3.75, 4.5, 10.0]
    values = (ctypes.c_double * len(input_values))(*input_values)
    actual = lib.sum_f64(values, ctypes.c_int32(len(input_values)))
    expected = sum(input_values)
    if not close(actual, expected):
        raise AssertionError(f"c-f64-sum mismatch actual={actual} expected={expected}")
    return f"c-f64-sum:result={format_float(actual)};inputLength={len(input_values)}"


def run_c_dijkstra(library_path: str) -> str:
    lib = ctypes.CDLL(library_path)
    lib.dijkstra_matrix.argtypes = [
        ctypes.POINTER(DijkstraConfig),
        ctypes.POINTER(ctypes.c_int64),
        ctypes.POINTER(ctypes.c_int64),
        ctypes.POINTER(ctypes.c_int32),
        ctypes.POINTER(ctypes.c_int32),
    ]
    lib.dijkstra_matrix.restype = ctypes.c_int32

    node_count = 5
    inf = 1_000_000
    config = DijkstraConfig(node_count=node_count, source=0, inf=inf)
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
            "c-dijkstra mismatch "
            f"settled={settled} dist={actual_dist} prev={actual_prev} visited={actual_visited}"
        )
    return (
        f"c-dijkstra:settled={settled};"
        f"dist={','.join(str(value) for value in actual_dist)};"
        f"prev={','.join(str(value) for value in actual_prev)};"
        f"visited={','.join(str(value) for value in actual_visited)}"
    )


def run_c_scalar_checked(library_path: str) -> str:
    lib = ctypes.CDLL(library_path)
    for name in ["add_i64", "mul_i64", "div_i64"]:
        fn = getattr(lib, name)
        fn.argtypes = [ctypes.c_int64, ctypes.c_int64, ctypes.POINTER(ctypes.c_int64)]
        fn.restype = ctypes.c_int32
    lib.neg_i64.argtypes = [ctypes.c_int64, ctypes.POINTER(ctypes.c_int64)]
    lib.neg_i64.restype = ctypes.c_int32
    lib.less_i64.argtypes = [ctypes.c_int64, ctypes.c_int64, ctypes.POINTER(ctypes.c_bool)]
    lib.less_i64.restype = ctypes.c_int32

    add_status, add = checked_i64_call(lib.add_i64, 2, 3)
    mul_status, product = checked_i64_call(lib.mul_i64, 3, 4)
    div_status, quotient = checked_i64_call(lib.div_i64, 10, 2)
    less_status, less = checked_bool_call(lib.less_i64, 1, 2)
    overflow_status, _ = checked_i64_call(lib.add_i64, 9223372036854775807, 1)
    div_zero_status, _ = checked_i64_call(lib.div_i64, 10, 0)
    neg_overflow_status, _ = checked_i64_call(lib.neg_i64, -9223372036854775808)
    if (
        add_status != CK_OK
        or add != 5
        or mul_status != CK_OK
        or product != 12
        or div_status != CK_OK
        or quotient != 5
        or less_status != CK_OK
        or less is not True
        or overflow_status != CK_ERR_OVERFLOW
        or div_zero_status != CK_ERR_DIV_BY_ZERO
        or neg_overflow_status != CK_ERR_OVERFLOW
    ):
        raise AssertionError("c-scalar-checked mismatch")
    return (
        f"c-scalar-checked:add={add};mul={product};div={quotient};less={int(less)};"
        f"overflow={overflow_status};div_zero={div_zero_status};neg_overflow={neg_overflow_status}"
    )


def run_c_control_checked(library_path: str) -> str:
    lib = ctypes.CDLL(library_path)
    for name in ["sum_to_n", "choose", "condition_overflow"]:
        fn = getattr(lib, name)
        if name == "sum_to_n":
            fn.argtypes = [ctypes.c_int64, ctypes.POINTER(ctypes.c_int64)]
        else:
            fn.argtypes = [ctypes.c_int64, ctypes.c_int64, ctypes.POINTER(ctypes.c_int64)]
        fn.restype = ctypes.c_int32

    sum_status, total = checked_i64_call(lib.sum_to_n, 5)
    high_status, high = checked_i64_call(lib.choose, 10, 3)
    low_status, low = checked_i64_call(lib.choose, 1, 3)
    overflow_status, _ = checked_i64_call(lib.condition_overflow, 9223372036854775807, 1)
    if (
        sum_status != CK_OK
        or total != 10
        or high_status != CK_OK
        or high != 10
        or low_status != CK_OK
        or low != 3
        or overflow_status != CK_ERR_OVERFLOW
    ):
        raise AssertionError("c-control-checked mismatch")
    return f"c-control-checked:sum={total};choose={high},{low};overflow={overflow_status}"


def run_c_logical_checked(library_path: str) -> str:
    lib = ctypes.CDLL(library_path)
    lib.and_short_circuit.argtypes = [ctypes.c_int64, ctypes.c_int64, ctypes.POINTER(ctypes.c_bool)]
    lib.and_short_circuit.restype = ctypes.c_int32
    lib.or_short_circuit.argtypes = [ctypes.c_int64, ctypes.c_int64, ctypes.POINTER(ctypes.c_bool)]
    lib.or_short_circuit.restype = ctypes.c_int32
    lib.and_rhs_error.argtypes = [ctypes.c_bool, ctypes.c_int64, ctypes.c_int64, ctypes.POINTER(ctypes.c_bool)]
    lib.and_rhs_error.restype = ctypes.c_int32

    cases = [
        checked_bool_call(lib.and_short_circuit, 0, 10),
        checked_bool_call(lib.and_short_circuit, 2, 10),
        checked_bool_call(lib.or_short_circuit, 0, 10),
        checked_bool_call(lib.or_short_circuit, 2, 10),
        checked_bool_call(lib.and_rhs_error, False, 10, 0),
    ]
    div_zero_status, _ = checked_bool_call(lib.and_rhs_error, True, 10, 0)
    expected = [
        (CK_OK, False),
        (CK_OK, True),
        (CK_OK, True),
        (CK_OK, True),
        (CK_OK, False),
    ]
    if cases != expected or div_zero_status != CK_ERR_DIV_BY_ZERO:
        raise AssertionError(f"c-logical-checked mismatch cases={cases} div_zero={div_zero_status}")
    encoded = ",".join(str(int(value)) for _, value in cases)
    return f"c-logical-checked:out={encoded};div_zero={div_zero_status}"


def run_c_calls_checked(library_path: str) -> str:
    lib = ctypes.CDLL(library_path)
    lib.calc.argtypes = [ctypes.c_int64, ctypes.c_int64, ctypes.POINTER(ctypes.c_int64)]
    lib.calc.restype = ctypes.c_int32
    lib.calc_overflow.argtypes = [ctypes.c_int64, ctypes.c_int64, ctypes.POINTER(ctypes.c_int64)]
    lib.calc_overflow.restype = ctypes.c_int32

    status, result = checked_i64_call(lib.calc, 1, 2)
    overflow_status, _ = checked_i64_call(lib.calc_overflow, 9223372036854775807, 1)
    if status != CK_OK or result != 6 or overflow_status != CK_ERR_OVERFLOW:
        raise AssertionError(f"c-calls-checked mismatch status={status} result={result} overflow={overflow_status}")
    return f"c-calls-checked:calc={result};overflow={overflow_status}"


def expected_total(price: int, quantity: int, discount: int, tax_rate_ppm: int) -> int:
    after_discount = price * quantity - discount
    tax = after_discount * tax_rate_ppm // 1000000
    return after_discount + tax


def run_c_perf_pricing_helpers(library_path: str, label: str) -> str:
    class Item(ctypes.Structure):
        _fields_ = [
            ("price", ctypes.c_int64),
            ("qty", ctypes.c_int64),
            ("discount", ctypes.c_int64),
            ("tax_rate_ppm", ctypes.c_int64),
        ]

    lib = ctypes.CDLL(library_path)
    lib.calc_items.argtypes = [
        ctypes.POINTER(Item),
        ctypes.c_int32,
        ctypes.POINTER(ctypes.c_int64),
    ]
    lib.calc_items.restype = ctypes.c_int32

    rows = [
        (10000, 2, 1000, 82500),
        (2500, 4, 0, 100000),
        (1200, 5, 500, 100000),
        (999, 3, 100, 62500),
    ]
    items = (Item * len(rows))(*(Item(*row) for row in rows))
    out = (ctypes.c_int64 * len(rows))()
    status = lib.calc_items(items, ctypes.c_int32(len(rows)), out)
    actual = list(out)
    expected = [expected_total(*row) for row in rows]
    if status != 0 or actual != expected:
        raise AssertionError(f"{label} mismatch status={status} actual={actual} expected={expected}")
    return f"{label}:status={status};out={','.join(str(value) for value in actual)}"


def run_c_perf_pricing_soa(library_path: str) -> str:
    lib = ctypes.CDLL(library_path)
    lib.pricing_soa.argtypes = [
        ctypes.POINTER(ctypes.c_int64),
        ctypes.POINTER(ctypes.c_int64),
        ctypes.POINTER(ctypes.c_int64),
        ctypes.POINTER(ctypes.c_int64),
        ctypes.POINTER(ctypes.c_int64),
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
        raise AssertionError(f"c-perf-pricing-soa-o3 mismatch status={status} actual={actual} expected={expected}")
    return f"c-perf-pricing-soa-o3:status={status};out={','.join(str(value) for value in actual)}"


def run_c_pricing_soa(library_path: str) -> str:
    lib = ctypes.CDLL(library_path)
    lib.pricing_soa.argtypes = [
        ctypes.POINTER(ctypes.c_int64),
        ctypes.POINTER(ctypes.c_int64),
        ctypes.POINTER(ctypes.c_int64),
        ctypes.POINTER(ctypes.c_int64),
        ctypes.POINTER(ctypes.c_int64),
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
        raise AssertionError(f"c-pricing-soa mismatch status={status} actual={actual} expected={expected}")
    return f"c-pricing-soa:status={status};out={','.join(str(value) for value in actual)}"


def run_c_wasm_scalar(library_path: str) -> str:
    lib = ctypes.CDLL(library_path)
    lib.add_i32.argtypes = [ctypes.c_int32, ctypes.c_int32]
    lib.add_i32.restype = ctypes.c_int32
    lib.add_i64.argtypes = [ctypes.c_int64, ctypes.c_int64]
    lib.add_i64.restype = ctypes.c_int64
    lib.less_i64.argtypes = [ctypes.c_int64, ctypes.c_int64]
    lib.less_i64.restype = ctypes.c_bool
    lib.div_u64.argtypes = [ctypes.c_uint64, ctypes.c_uint64]
    lib.div_u64.restype = ctypes.c_uint64

    add_i32 = lib.add_i32(1, 2)
    add_i64 = lib.add_i64(1, 2)
    less_i64 = lib.less_i64(1, 2)
    div_u64 = lib.div_u64(10, 2)
    if add_i32 != 3 or add_i64 != 3 or less_i64 is not True or div_u64 != 5:
        raise AssertionError(
            f"c-wasm-scalar mismatch add_i32={add_i32} add_i64={add_i64} less_i64={less_i64} div_u64={div_u64}"
        )
    return f"c-wasm-scalar:add_i32={add_i32};add_i64={add_i64};less_i64={int(less_i64)};div_u64={div_u64}"


def run_c_wasm_calls(library_path: str) -> str:
    lib = ctypes.CDLL(library_path)
    lib.calc.argtypes = [ctypes.c_int64, ctypes.c_int64]
    lib.calc.restype = ctypes.c_int64

    result = lib.calc(1, 2)
    if result != 6:
        raise AssertionError(f"c-wasm-calls mismatch result={result}")
    return f"c-wasm-calls:calc={result}"


def run_c_wasm_control_flow(library_path: str) -> str:
    lib = ctypes.CDLL(library_path)
    lib.max_i32.argtypes = [ctypes.c_int32, ctypes.c_int32]
    lib.max_i32.restype = ctypes.c_int32
    lib.sum_to_n.argtypes = [ctypes.c_int64]
    lib.sum_to_n.restype = ctypes.c_int64

    high = lib.max_i32(10, 3)
    low = lib.max_i32(1, 3)
    total = lib.sum_to_n(5)
    if high != 10 or low != 3 or total != 10:
        raise AssertionError(f"c-wasm-control-flow mismatch high={high} low={low} total={total}")
    return f"c-wasm-control-flow:max={high},{low};sum={total}"


def run_c_wasm_memory(library_path: str) -> str:
    class Item(ctypes.Structure):
        _fields_ = [
            ("price", ctypes.c_int64),
            ("qty", ctypes.c_int64),
            ("discount", ctypes.c_int64),
            ("tax_rate_ppm", ctypes.c_int64),
        ]

    lib = ctypes.CDLL(library_path)
    lib.first_price.argtypes = [ctypes.POINTER(Item)]
    lib.first_price.restype = ctypes.c_int64
    lib.get_price.argtypes = [ctypes.POINTER(Item), ctypes.c_int32]
    lib.get_price.restype = ctypes.c_int64
    lib.write_i64.argtypes = [ctypes.POINTER(ctypes.c_int64), ctypes.c_int64]
    lib.write_i64.restype = ctypes.c_int32

    items = (Item * 2)(
        Item(price=1234, qty=2, discount=3, tax_rate_ppm=4),
        Item(price=222, qty=0, discount=0, tax_rate_ppm=0),
    )
    out = (ctypes.c_int64 * 1)(0)
    first = lib.first_price(items)
    indexed = lib.get_price(items, ctypes.c_int32(1))
    status = lib.write_i64(out, ctypes.c_int64(123))
    stored = out[0]
    if first != 1234 or indexed != 222 or status != 0 or stored != 123:
        raise AssertionError(
            f"c-wasm-memory mismatch first={first} indexed={indexed} status={status} stored={stored}"
        )
    return f"c-wasm-memory:first={first};indexed={indexed};status={status};stored={stored}"


def run_c_wasm_short_circuit(library_path: str) -> str:
    lib = ctypes.CDLL(library_path)
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
        raise AssertionError(f"c-wasm-short-circuit mismatch values={values} expected={expected}")
    encoded = ",".join(str(int(value)) for value in values)
    return f"c-wasm-short-circuit:out={encoded}"


def run_c_llvm_scalar(library_path: str) -> str:
    lib = ctypes.CDLL(library_path)
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
            f"c-llvm-scalar mismatch add_i64={add_i64} mul_i32={mul_i32} "
            f"less_i64={less_i64} div_u64={div_u64}"
        )
    return f"c-llvm-scalar:add_i64={add_i64};mul_i32={mul_i32};less_i64={int(less_i64)};div_u64={div_u64}"


def run_c_llvm_calls(library_path: str) -> str:
    lib = ctypes.CDLL(library_path)
    lib.calc.argtypes = [ctypes.c_int64, ctypes.c_int64]
    lib.calc.restype = ctypes.c_int64

    result = lib.calc(1, 2)
    if result != 6:
        raise AssertionError(f"c-llvm-calls mismatch result={result}")
    return f"c-llvm-calls:calc={result}"


def run_c_llvm_control_flow(library_path: str) -> str:
    lib = ctypes.CDLL(library_path)
    lib.max_i32.argtypes = [ctypes.c_int32, ctypes.c_int32]
    lib.max_i32.restype = ctypes.c_int32
    lib.sum_to_n.argtypes = [ctypes.c_int64]
    lib.sum_to_n.restype = ctypes.c_int64

    high = lib.max_i32(10, 3)
    low = lib.max_i32(1, 3)
    total = lib.sum_to_n(5)
    if high != 10 or low != 3 or total != 10:
        raise AssertionError(f"c-llvm-control-flow mismatch high={high} low={low} total={total}")
    return f"c-llvm-control-flow:max={high},{low};sum={total}"


def run_c_llvm_memory(library_path: str) -> str:
    class Item(ctypes.Structure):
        _fields_ = [
            ("price", ctypes.c_int64),
            ("qty", ctypes.c_int64),
            ("discount", ctypes.c_int64),
            ("tax_rate_ppm", ctypes.c_int64),
        ]

    lib = ctypes.CDLL(library_path)
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
            f"c-llvm-memory mismatch first={first} indexed={indexed} "
            f"status={status} stored={stored}"
        )
    return f"c-llvm-memory:first={first};indexed={indexed};status={status};stored={stored}"


def run_c_llvm_short_circuit(library_path: str) -> str:
    lib = ctypes.CDLL(library_path)
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
        raise AssertionError(f"c-llvm-short-circuit mismatch values={values} expected={expected}")
    encoded = ",".join(str(int(value)) for value in values)
    return f"c-llvm-short-circuit:out={encoded}"


def run_c_llvm_bool(library_path: str) -> str:
    lib = ctypes.CDLL(library_path)
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
            "c-llvm-bool mismatch "
            f"not={not_true},{not_false} local={local_true},{local_false} "
            f"choose={choose_true},{choose_false}"
        )
    return (
        f"c-llvm-bool:not={int(not_true)},{int(not_false)};"
        f"local={int(local_true)},{int(local_false)};"
        f"choose={choose_true},{choose_false}"
    )


def run_c_perf_f64_kernels(library_path: str) -> str:
    lib = ctypes.CDLL(library_path)
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
    if not close(axpy_checksum, axpy_expected_checksum) or any(
        not close(actual, expected) for actual, expected in zip(axpy_actual, axpy_expected)
    ):
        raise AssertionError(f"c-perf-f64-kernels-o3 axpy mismatch checksum={axpy_checksum} out={axpy_actual}")

    x = (ctypes.c_double * length)(*x_input)
    y = (ctypes.c_double * length)(*y_input)
    dot_actual = lib.dot_f64(x, y, ctypes.c_int32(length))
    dot_expected = sum(value * y_input[index] for index, value in enumerate(x_input))
    if not close(dot_actual, dot_expected):
        raise AssertionError(f"c-perf-f64-kernels-o3 dot mismatch actual={dot_actual} expected={dot_expected}")

    sum_actual = lib.sum_f64(x, ctypes.c_int32(length))
    sum_expected = sum(x_input)
    if not close(sum_actual, sum_expected):
        raise AssertionError(f"c-perf-f64-kernels-o3 sum mismatch actual={sum_actual} expected={sum_expected}")

    scale_input = [0.25, -1.5, 2.0, 10.0]
    scale = (ctypes.c_double * length)(*scale_input)
    scale_checksum = lib.scale_f64(ctypes.c_double(-2.0), scale, ctypes.c_int32(length))
    scale_actual = list(scale)
    scale_expected = [-2.0 * value for value in scale_input]
    scale_expected_checksum = sum(scale_expected)
    if not close(scale_checksum, scale_expected_checksum) or any(
        not close(actual, expected) for actual, expected in zip(scale_actual, scale_expected)
    ):
        raise AssertionError(f"c-perf-f64-kernels-o3 scale mismatch checksum={scale_checksum} out={scale_actual}")

    return (
        f"c-perf-f64-kernels-o3:axpy={format_float(axpy_checksum)};"
        f"dot={format_float(dot_actual)};sum={format_float(sum_actual)};"
        f"scale={format_float(scale_checksum)};"
        f"axpyOut={','.join(format_float(value) for value in axpy_actual)};"
        f"scaleOut={','.join(format_float(value) for value in scale_actual)}"
    )


def run_c_f64_edges(library_path: str) -> str:
    lib = ctypes.CDLL(library_path)
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
        ("finite_add", close(lib.finite_add(), 4.0)),
        ("finite_sub", close(lib.finite_sub(), 3.5)),
        ("finite_mul", close(lib.finite_mul(), 3.75)),
        ("finite_div", close(lib.finite_div(), 3.5)),
        ("tolerance_calc", close(lib.tolerance_calc(), 10.0)),
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
        ("ptr_load", close(lib.ptr_read(values, ctypes.c_int32(2)), 4.0)),
        ("ptr_store", close(ptr_store_value, 8.75) and close(values[1], 8.75)),
        ("struct_read", close(lib.struct_read(quotes, ctypes.c_int32(0)), 11.0)),
        ("struct_write", close(struct_write_value, 21.0) and close(quotes[1].tax, 0.5)),
        ("nested_struct_read", close(lib.nested_struct_read(nested, ctypes.c_int32(0)), 4.0)),
        (
            "nested_struct_write",
            close(nested_write_value, 14.5) and close(nested[1].quote.tax, 1.5),
        ),
    ]
    return "c-f64-edges:" + ";".join(f"{name}={ok(passed)}" for name, passed in checks)


RUNNERS = {
    "c-scalar": run_c_scalar,
    "c-casts": run_c_casts,
    "c-f64-array": run_c_f64_array,
    "c-f64-axpy": run_c_f64_axpy,
    "c-f64-sum": run_c_f64_sum,
    "c-pricing-soa": run_c_pricing_soa,
    "c-wasm-scalar": run_c_wasm_scalar,
    "c-wasm-calls": run_c_wasm_calls,
    "c-wasm-control-flow": run_c_wasm_control_flow,
    "c-wasm-memory": run_c_wasm_memory,
    "c-wasm-short-circuit": run_c_wasm_short_circuit,
    "c-llvm-scalar": run_c_llvm_scalar,
    "c-llvm-calls": run_c_llvm_calls,
    "c-llvm-control-flow": run_c_llvm_control_flow,
    "c-llvm-memory": run_c_llvm_memory,
    "c-llvm-short-circuit": run_c_llvm_short_circuit,
    "c-llvm-bool": run_c_llvm_bool,
    "c-dijkstra": run_c_dijkstra,
    "c-scalar-checked": run_c_scalar_checked,
    "c-control-checked": run_c_control_checked,
    "c-logical-checked": run_c_logical_checked,
    "c-calls-checked": run_c_calls_checked,
    "c-perf-pricing-helpers-o0": lambda path: run_c_perf_pricing_helpers(path, "c-perf-pricing-helpers-o0"),
    "c-perf-pricing-helpers-o2": lambda path: run_c_perf_pricing_helpers(path, "c-perf-pricing-helpers-o2"),
    "c-perf-pricing-soa-o3": run_c_perf_pricing_soa,
    "c-perf-f64-kernels-o3": run_c_perf_f64_kernels,
    "c-f64-edges": run_c_f64_edges,
}


def main() -> None:
    case_name, library_path = sys.argv[1:3]
    runner = RUNNERS.get(case_name)
    if runner is None:
        raise RuntimeError(f"unknown C dynamic case: {case_name}")
    print(runner(library_path))


if __name__ == "__main__":
    main()
"#
}
