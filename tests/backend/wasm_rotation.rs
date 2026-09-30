use calckernel::{
    BoundsMode, EmitWasmOptions, KirConsumer, KirInstructionKind, KirTerminator, MirCompareOp,
    OverflowMode, emit_wasm_kir_module, emit_wat_kir_module, validate_kir_module,
};

use super::support::command::node_available;
use super::support::compiler::{optimized_module, verified_artifact};
use std::{fs, process::Command};

const LOOP_SOURCE: &str = r#"
export fn rotate_values(n: u32, seed: u32) -> u32 {
  let i: u32 = 0;
  let left: u32 = seed;
  let right: u32 = 1;
  if seed == 0 { left = 1; }
  while i < n {
    let previous: u32 = left;
    left = right;
    right = previous + left;
    i = i + 1;
  }
  return left + right;
}
"#;

fn optimized_wasm(source: &str, opt_level: u8) -> (String, Vec<u8>) {
    let optimized = optimized_module(
        source,
        opt_level,
        KirConsumer::WebAssembly,
        OverflowMode::Unchecked,
        BoundsMode::Unchecked,
    );
    let artifact = verified_artifact(&optimized);
    let options = EmitWasmOptions { opt_level };
    (
        emit_wat_kir_module(artifact, options).expect("verified Wasm KIR lowers to WAT"),
        emit_wasm_kir_module(artifact, options).expect("verified Wasm KIR lowers to bytes"),
    )
}

fn run_node(bytes: &[u8], runner: &str) -> Option<String> {
    if !node_available() {
        return None;
    }
    let directory = super::support::temp::temp_dir("ck-wasm-rotation");
    fs::create_dir_all(&directory).expect("create test directory");
    let wasm_path = directory.join("module.wasm");
    let runner_path = directory.join("runner.cjs");
    fs::write(&wasm_path, bytes).expect("write Wasm fixture");
    fs::write(&runner_path, runner).expect("write Node runner");
    let output = Command::new("node")
        .arg(&runner_path)
        .arg(&wasm_path)
        .output()
        .expect("run Node Wasm harness");
    let _ = fs::remove_dir_all(&directory);
    assert!(
        output.status.success(),
        "Node Wasm harness failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    Some(String::from_utf8(output.stdout).expect("Node output is UTF-8"))
}

#[test]
fn wasm_loop_rotation_preserves_zero_one_many_iterations_and_loop_carried_copies() {
    let (wat, wasm) = optimized_wasm(LOOP_SOURCE, 3);
    let loop_body = wat
        .split("(func $rotate_values")
        .nth(1)
        .expect("exported function");
    assert_eq!(
        loop_body.matches("i32.lt_u").count(),
        2,
        "condition is evaluated initially and after each body:\n{wat}"
    );
    assert!(
        loop_body.contains("br_if $ik_loop_"),
        "rotated backedge is conditional:\n{wat}"
    );
    let wat_binary = wat::parse_str(&wat).expect("rotated WAT parses to a module");

    let runner = r#"
const fs = require("node:fs");
WebAssembly.instantiate(fs.readFileSync(process.argv[2])).then(({instance}) => {
  const f = instance.exports.rotate_values;
  const actual = [f(0, 10), f(1, 10), f(2, 10), f(5, 10)];
  const expected = [11, 12, 23, 93];
  if (actual.some((value, index) => value !== expected[index])) {
    throw new Error(`rotation results ${actual}; expected ${expected}`);
  }
  process.stdout.write(actual.join(","));
}).catch(error => { console.error(error); process.exitCode = 1; });
"#;
    let Some(direct_actual) = run_node(&wasm, runner) else {
        return;
    };
    let wat_actual = run_node(&wat_binary, runner).expect("Node available for WAT oracle");
    assert_eq!(direct_actual, "11,12,23,93");
    assert_eq!(
        wat_actual, direct_actual,
        "direct binary and WAT module agree"
    );
}

#[test]
fn wasm_loop_rotation_inverts_a_false_body_branch_without_changing_loop_results() {
    let optimized = optimized_module(
        LOOP_SOURCE,
        3,
        KirConsumer::WebAssembly,
        OverflowMode::Unchecked,
        BoundsMode::Unchecked,
    );
    let mut artifact = verified_artifact(&optimized).clone();
    let function = artifact
        .functions
        .iter_mut()
        .find(|function| function.name == "rotate_values")
        .expect("rotate_values function");
    let mut inverted = false;
    for block in &mut function.blocks {
        if let KirTerminator::Branch {
            then_edge,
            else_edge,
            ..
        } = &mut block.terminator
        {
            let is_loop_header = block.instructions.iter_mut().any(|instruction| {
                if let KirInstructionKind::Compare { op, .. } = &mut instruction.kind
                    && *op == MirCompareOp::Lt
                {
                    *op = MirCompareOp::Ge;
                    true
                } else {
                    false
                }
            });
            if is_loop_header {
                std::mem::swap(then_edge, else_edge);
                inverted = true;
                break;
            }
        }
    }
    assert!(inverted, "find the top-tested unsigned less-than branch");
    let diagnostics = validate_kir_module(&artifact);
    assert_eq!(diagnostics.errors, [], "branch inversion remains valid KIR");

    let options = EmitWasmOptions { opt_level: 3 };
    let wat = emit_wat_kir_module(&artifact, options).expect("inverted KIR lowers to WAT");
    let body = wat
        .split("(func $rotate_values")
        .nth(1)
        .expect("exported function");
    assert!(
        body.contains("i32.eqz\n"),
        "false body arm uses zero inversion:\n{wat}"
    );
    assert_eq!(
        body.matches("i32.ge_u").count(),
        2,
        "rotated condition remains paired with the inversion:\n{wat}"
    );

    let wasm = emit_wasm_kir_module(&artifact, options).expect("inverted KIR lowers to bytes");
    let wat_binary = wat::parse_str(&wat).expect("inverted WAT parses to a module");
    let runner = r#"
const fs = require("node:fs");
WebAssembly.instantiate(fs.readFileSync(process.argv[2])).then(({instance}) => {
  const f = instance.exports.rotate_values;
  const actual = [f(0, 10), f(1, 10), f(2, 10), f(5, 10)];
  const expected = [11, 12, 23, 93];
  if (actual.some((value, index) => value !== expected[index])) {
    throw new Error(`inverted rotation results ${actual}; expected ${expected}`);
  }
  process.stdout.write(actual.join(","));
}).catch(error => { console.error(error); process.exitCode = 1; });
"#;
    let Some(direct_actual) = run_node(&wasm, runner) else {
        return;
    };
    let wat_actual = run_node(&wat_binary, runner).expect("Node available for WAT oracle");
    assert_eq!(direct_actual, "11,12,23,93");
    assert_eq!(
        wat_actual, direct_actual,
        "direct binary and WAT module agree"
    );
}

#[test]
fn wasm_loop_rotation_is_limited_to_the_o3_structured_emitter() {
    for opt_level in 0..=2 {
        let (wat, _) = optimized_wasm(LOOP_SOURCE, opt_level);
        let body = wat
            .split("(func $rotate_values")
            .nth(1)
            .expect("exported function");
        assert!(
            !body.contains("loop $ik_loop_"),
            "O{opt_level} must retain the existing dispatcher/legacy loop path:\n{wat}"
        );
        assert_eq!(
            body.matches("i32.lt_u").count(),
            1,
            "O{opt_level} must not duplicate the loop condition:\n{wat}"
        );
    }
}

#[test]
fn wasm_loop_rotation_falls_back_for_header_traps_and_side_exits() {
    let trapping_header = r#"
export fn divide_in_header(divisor: u32, limit: u32) -> u32 {
  let i: u32 = 0;
  while (10 / divisor) > i && i < limit {
    i = i + 1;
  }
  return i;
}
"#;
    let (trap_wat, trap_wasm) = optimized_wasm(trapping_header, 3);
    let trap_body = trap_wat
        .split("(func $divide_in_header")
        .nth(1)
        .expect("exported function");
    assert_eq!(
        trap_body.matches("i32.div_u").count(),
        1,
        "trapping header remains in the original loop:\n{trap_wat}"
    );
    assert_eq!(
        trap_body.matches("i32.lt_u").count(),
        1,
        "header comparison is not duplicated:\n{trap_wat}"
    );

    let Some(trap_result) = run_node(
        &trap_wasm,
        r#"
const fs = require("node:fs");
WebAssembly.instantiate(fs.readFileSync(process.argv[2])).then(({instance}) => {
  let trapped = false;
  try { instance.exports.divide_in_header(0, 8); } catch (_) { trapped = true; }
  if (!trapped) throw new Error("zero divisor should trap before entering the loop");
  process.stdout.write("trapped");
}).catch(error => { console.error(error); process.exitCode = 1; });
"#,
    ) else {
        return;
    };
    assert_eq!(trap_result, "trapped");

    let side_exit = r#"
export fn stop_early(limit: u32) -> u32 {
  let i: u32 = 0;
  while i < limit {
    if i == 3 { break; }
    i = i + 1;
  }
  return i;
}
"#;
    let (side_exit_wat, _) = optimized_wasm(side_exit, 3);
    let side_exit_body = side_exit_wat
        .split("(func $stop_early")
        .nth(1)
        .expect("exported function");
    assert_eq!(
        side_exit_body.matches("i32.lt_u").count(),
        1,
        "side-exit loop retains the original shape:\n{side_exit_wat}"
    );
}
