use std::{fs, process::Command};

use calckernel::{
    EmitWasmOptions, KirBoundsMode, KirBuildConfig, KirConsumer, KirInstructionKind,
    KirOptimizationLevel, KirOverflowMode, KirSanitizerMode, SourceFile, build_kir_module, check,
    emit_wasm_kir_module, emit_wat_kir_module, import_contract_facts, lower_to_mir,
    run_kir_pass_pipeline,
};

use crate::generated::fixed_seed_kernel_program;
use crate::support::temp::temp_dir;

fn optimized_kir(source: &str, level: KirOptimizationLevel) -> calckernel::KirModule {
    let checked = check(&SourceFile::new("kir-wasm.ck", source));
    assert_eq!(checked.diagnostics, []);
    let mir = lower_to_mir(&checked.checked_program).expect("MIR");
    let kir = build_kir_module(
        &mir,
        KirBuildConfig {
            consumer: KirConsumer::WebAssembly,
            overflow_mode: KirOverflowMode::Unchecked,
            bounds_mode: KirBoundsMode::Unchecked,
            sanitizer_mode: KirSanitizerMode::Disabled,
        },
    )
    .expect("KIR");
    let contracts = checked
        .checked_program
        .functions
        .iter()
        .any(|function| function.is_unsafe)
        .then(|| import_contract_facts(&kir, &checked.checked_program, 0).expect("contract facts"));
    let result = run_kir_pass_pipeline(kir, level, contracts.as_ref());
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    result.artifact.expect("artifact")
}

#[test]
fn kir_wasm_backend_should_emit_validate_and_run_control_flow() {
    let kir = optimized_kir(
        r#"
        export fn sum(n: i32) -> i32 {
          let i: i32 = 0; let total: i32 = 0;
          while i < n { total = total + i; i = i + 1; }
          return total;
        }
        "#,
        KirOptimizationLevel::O3,
    );
    let options = EmitWasmOptions { opt_level: 3 };
    let wat = emit_wat_kir_module(&kir, options).expect("KIR WAT");
    let wasm = emit_wasm_kir_module(&kir, options).expect("KIR WASM");
    assert!(wat.contains("(export \"sum\")"));
    assert_eq!(&wasm[..8], b"\0asm\x01\0\0\0");

    let temp = temp_dir("kir_wasm_backend");
    fs::create_dir_all(&temp).expect("create temp dir");
    let wasm_path = temp.join("case.wasm");
    let runner = temp.join("run.mjs");
    fs::write(&wasm_path, wasm).expect("write wasm");
    fs::write(
        &runner,
        r#"
        import fs from "node:fs";
        const bytes = fs.readFileSync(process.argv[2]);
        const { instance } = await WebAssembly.instantiate(bytes, {});
        if (instance.exports.sum(5) !== 10) process.exit(1);
        "#,
    )
    .expect("write runner");
    let output = Command::new("node")
        .arg(&runner)
        .arg(&wasm_path)
        .output()
        .expect("node");
    assert!(
        output.status.success(),
        "node failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    fs::remove_dir_all(temp).expect("remove temp dir");
}

#[test]
fn kir_wasm_backend_should_reject_checked_kir_without_inventing_an_abi() {
    let checked = check(&SourceFile::new(
        "checked.ck",
        "export fn add(a: i32, b: i32) -> i32 { return a + b; }",
    ));
    let mir = lower_to_mir(&checked.checked_program).expect("MIR");
    let kir = build_kir_module(
        &mir,
        KirBuildConfig {
            consumer: KirConsumer::Inspection,
            overflow_mode: KirOverflowMode::Checked,
            bounds_mode: KirBoundsMode::Checked,
            sanitizer_mode: KirSanitizerMode::Disabled,
        },
    )
    .expect("KIR");
    let error = emit_wat_kir_module(&kir, EmitWasmOptions::default()).expect_err("reject");
    assert!(error.contains("only unchecked KIR"));
}

#[test]
fn kir_wasm_licm_should_not_introduce_traps_on_zero_trip_or_break_paths() {
    let source = r#"
        export fn divide(a: i32, d: i32, n: u32) -> i32 {
          let i: u32 = 0; let total: i32 = 0;
          while i < n { total = total + a / d; i = i + 1; }
          return total;
        }
        export fn remainder(a: i32, d: i32, n: u32) -> i32 {
          let i: u32 = 0; let total: i32 = 0;
          while i < n { if d == 0 { break; } total = total + a % d; i = i + 1; }
          return total;
        }
    "#;
    let runner = r#"
        import fs from "node:fs";
        const { instance } = await WebAssembly.instantiate(fs.readFileSync(process.argv[2]), {});
        const api = instance.exports;
        if (api.divide(1, 0, 0) !== 0) process.exit(1);
        if (api.divide(-2147483648, -1, 0) !== 0) process.exit(2);
        if (api.remainder(1, 0, 0) !== 0) process.exit(3);
        if (api.remainder(1, 0, 1) !== 0) process.exit(4);
        if (api.divide(12, 3, 2) !== 8) process.exit(5);
        if (api.remainder(13, 3, 2) !== 2) process.exit(6);
    "#;
    for (level, kir_level) in [
        (0, KirOptimizationLevel::O0),
        (1, KirOptimizationLevel::O1),
        (2, KirOptimizationLevel::O2),
        (3, KirOptimizationLevel::O3),
    ] {
        let kir = optimized_kir(source, kir_level);
        let wasm = emit_wasm_kir_module(&kir, EmitWasmOptions { opt_level: level }).expect("WASM");
        run_wasm(&wasm, runner);
    }
}

#[test]
fn kir_wasm_o0_through_o3_should_cover_supported_mode_matrix() {
    const SOURCE: &str = r#"
        struct Pair { x: i32; y: i32; }
        export fn scalar(a: i32, b: i32) -> i32 { return a * 3 + b; }
        export fn control(n: i32) -> i32 {
          let i: i32 = 0; let total: i32 = 0;
          while i < n { total = total + i; i = i + 1; }
          return total;
        }
        export fn write(out: ptr<i32>, value: i32) -> void { out[0] = value; }
        export fn slice_total(items: slice<i32>) -> i32 { return items[0] + items[1]; }
        export fn pair_total(pair: ptr<Pair>) -> i32 { return pair[0].x + pair[0].y; }
    "#;
    let runner_source = r#"
        import fs from "node:fs";
        const bytes = fs.readFileSync(process.argv[2]);
        const { instance } = await WebAssembly.instantiate(bytes, {});
        const api = instance.exports;
        const memory = new Int32Array(api.memory.buffer);
        const out = 64, items = 80, pair = 96;
        memory[items / 4] = 20; memory[items / 4 + 1] = 22;
        memory[pair / 4] = 19; memory[pair / 4 + 1] = 23;
        if (api.scalar(10, 12) !== 42) process.exit(1);
        if (api.control(10) !== 45) process.exit(2);
        api.write(out, 42); if (memory[out / 4] !== 42) process.exit(3);
        if (api.slice_total(items, 2) !== 42) process.exit(4);
        if (api.pair_total(pair) !== 42) process.exit(5);
    "#;

    for (level, kir_level) in [
        (0, KirOptimizationLevel::O0),
        (1, KirOptimizationLevel::O1),
        (2, KirOptimizationLevel::O2),
        (3, KirOptimizationLevel::O3),
    ] {
        let kir = optimized_kir(SOURCE, kir_level);
        let kir_wasm = emit_wasm_kir_module(&kir, EmitWasmOptions { opt_level: level })
            .expect("KIR WASM matrix");
        run_wasm(&kir_wasm, runner_source);
    }
}

#[test]
fn generated_wasm_kernels_should_match_o0_at_o1_through_o3_in_supported_mode() {
    let generated = fixed_seed_kernel_program();
    let mut runner = String::from(
        r#"
        import fs from "node:fs";
        const bytes = fs.readFileSync(process.argv[2]);
        const { instance } = await WebAssembly.instantiate(bytes, {});
        const api = instance.exports;
        const memory = new Int32Array(api.memory.buffer);
"#,
    );
    for (index, case) in generated.cases.iter().enumerate() {
        let pointer = 256 + index * 64;
        let values = case
            .values
            .iter()
            .map(i32::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        runner.push_str(&format!(
            "memory.set([{values}], {pointer} / 4);\nif (api.{}({pointer}, 8, {}, {}) !== {}) process.exit({});\n",
            case.function,
            case.len,
            case.bias,
            case.expected,
            index + 1,
        ));
    }

    for (level, kir_level) in [
        (0, KirOptimizationLevel::O0),
        (1, KirOptimizationLevel::O1),
        (2, KirOptimizationLevel::O2),
        (3, KirOptimizationLevel::O3),
    ] {
        let kir = optimized_kir(&generated.source, kir_level);
        let wasm = emit_wasm_kir_module(&kir, EmitWasmOptions { opt_level: level })
            .expect("generated KIR WASM");
        run_wasm(&wasm, &runner);
    }
}

#[test]
fn kir_wasm_o3_canonical_proof_loop_should_consume_guard_free_kir() {
    const SOURCE: &str = include_str!("../fixtures/performance/native/proof_loop.ck");
    let kir = optimized_kir(SOURCE, KirOptimizationLevel::O3);
    let guards = kir
        .functions
        .iter()
        .flat_map(|function| &function.blocks)
        .flat_map(|block| &block.instructions)
        .filter(|instruction| matches!(instruction.kind, KirInstructionKind::Guard { .. }))
        .count();
    assert_eq!(guards, 0, "unchecked proof-loop KIR retained a guard");

    let options = EmitWasmOptions { opt_level: 3 };
    let wat = emit_wat_kir_module(&kir, options).expect("proof-loop WAT");
    assert!(!wat.contains("unreachable"), "{wat}");
    assert!(!wat.contains("CK_ERR_OUT_OF_BOUNDS"), "{wat}");
    let wasm = emit_wasm_kir_module(&kir, options).expect("proof-loop WASM");
    run_wasm(
        &wasm,
        r#"
        import fs from "node:fs";
        const bytes = fs.readFileSync(process.argv[2]);
        const { instance } = await WebAssembly.instantiate(bytes, {});
        const values = new BigInt64Array(instance.exports.memory.buffer);
        values.set([3n, 42n, -5n, 11n], 256 / 8);
        if (instance.exports.kernel(256, 4, 7n) !== 42n) process.exit(1);
        "#,
    );
}

fn run_wasm(wasm: &[u8], runner_source: &str) {
    let temp = temp_dir("kir_wasm_matrix");
    fs::create_dir_all(&temp).expect("create temp dir");
    let wasm_path = temp.join("case.wasm");
    let runner = temp.join("run.mjs");
    fs::write(&wasm_path, wasm).expect("write wasm");
    fs::write(&runner, runner_source).expect("write runner");
    let output = Command::new("node")
        .arg(&runner)
        .arg(&wasm_path)
        .output()
        .expect("node");
    assert!(
        output.status.success(),
        "node failed with {output:?}:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    fs::remove_dir_all(temp).expect("remove temp dir");
}
