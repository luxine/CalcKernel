use calckernel::{
    BoundsMode, EmitWasmOptions, KirConsumer, OverflowMode, emit_wasm_kir_module,
    emit_wat_kir_module,
};

use super::support::{
    command::node_available,
    compiler::{optimized_module, verified_artifact},
};
use std::{fs, process::Command};

const PIECEWISE_SOURCE: &str = r#"
export fn piecewise(input: slice<f64>, out: slice<f64>) -> void {
  let i: u32 = 0;
  while i < input.len {
    let x: f64 = input[i];
    if x < -0.25 {
      out[i] = x * x + 0.5;
      i = i + 1;
      continue;
    }
    if x < 0.0 {
      out[i] = x * 0.75 - 0.125;
      i = i + 1;
      continue;
    }
    if x < 0.25 {
      out[i] = x * x * x + 0.25;
      i = i + 1;
      continue;
    }
    out[i] = (x - 0.25) * 1.5;
    i = i + 1;
  }
}
"#;

fn emitted_piecewise(source: &str, opt_level: u8) -> (String, Vec<u8>) {
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
        emit_wat_kir_module(artifact, options).expect("verified Wasm KIR emits WAT"),
        emit_wasm_kir_module(artifact, options).expect("verified Wasm KIR emits bytes"),
    )
}

fn piecewise_body(wat: &str) -> &str {
    let body = wat
        .split("(func $piecewise")
        .nth(1)
        .expect("exported piecewise function");
    body.split("\n  (func ").next().unwrap_or(body)
}

fn run_node(bytes: &[u8], baseline: Option<&[u8]>, runner: &str) -> Option<()> {
    if !node_available() {
        return None;
    }
    let directory = super::support::temp::temp_dir("ck-wasm-piecewise-select");
    fs::create_dir_all(&directory).expect("create temporary Wasm test directory");
    let wasm_path = directory.join("module.wasm");
    let baseline_path = directory.join("baseline.wasm");
    let runner_path = directory.join("runner.cjs");
    fs::write(&wasm_path, bytes).expect("write Wasm test module");
    if let Some(baseline) = baseline {
        fs::write(&baseline_path, baseline).expect("write baseline Wasm test module");
    }
    fs::write(&runner_path, runner).expect("write Node oracle");
    let mut command = Command::new("node");
    command.arg(&runner_path).arg(&wasm_path);
    if baseline.is_some() {
        command.arg(&baseline_path);
    }
    let output = command.output().expect("run Node Wasm oracle");
    let _ = fs::remove_dir_all(&directory);
    assert!(
        output.status.success(),
        "Node Wasm oracle failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    Some(())
}

#[test]
fn wasm_piecewise_closed_store_tree_should_emit_one_store_and_nested_selects() {
    let (wat, _wasm) = emitted_piecewise(PIECEWISE_SOURCE, 3);
    let body = piecewise_body(&wat);
    assert_eq!(
        body.matches("f64.store").count(),
        1,
        "all four same-index leaves should reach one shared store:\n{body}"
    );
    assert_eq!(
        body.matches("select\n").count(),
        3,
        "the four leaves should form a three-level scalar select tree:\n{body}"
    );
}

#[test]
fn wasm_piecewise_select_should_reject_excessive_speculative_leaf_work() {
    const MULS: usize = 128;
    let mut source = String::from(
        "export fn heavy_piecewise(input: slice<f64>, out: slice<f64>) -> void {\n\
         let i: u32 = 0;\n\
         while i < input.len {\n\
           let x: f64 = input[i];\n\
           if x < 0.0 {\n\
             let y0: f64 = x;\n",
    );
    for index in 0..MULS {
        source.push_str(&format!(
            "             let y{}: f64 = y{} * 1.0001;\n",
            index + 1,
            index
        ));
    }
    source.push_str(&format!(
        "             out[i] = y{MULS};\n\
         i = i + 1;\n\
         continue;\n\
       }}\n\
       out[i] = x;\n\
       i = i + 1;\n\
     }}\n\
   }}\n"
    ));
    let (wat, _) = emitted_piecewise(&source, 3);
    let body = wat
        .split("(func $heavy_piecewise")
        .nth(1)
        .expect("exported heavy_piecewise function")
        .split("\n  (func ")
        .next()
        .unwrap_or(&wat);
    assert!(
        body.matches("f64.store").count() > 1,
        "a branch with {MULS} speculative strict multiplies should retain separate stores; found {} stores and {} selects",
        body.matches("f64.store").count(),
        body.matches("select\n").count(),
    );
    assert!(
        body.contains("if\n"),
        "the heavy tree should fall back to branch emission:\n{body}"
    );
}

#[test]
fn wasm_piecewise_select_should_match_strict_f64_edges_and_aliasing() {
    let (wat, wasm) = emitted_piecewise(PIECEWISE_SOURCE, 3);
    let (_, baseline) = emitted_piecewise(PIECEWISE_SOURCE, 0);
    let wat_bytes = wat::parse_str(&wat).expect("piecewise WAT parses");
    let runner = r#"
const fs = require("node:fs");
const bytes = fs.readFileSync(process.argv[2]);
function bits(value) {
  const view = new DataView(new ArrayBuffer(8));
  view.setFloat64(0, value, true);
  return view.getBigUint64(0, true);
}
function fromBits(value) {
  const view = new DataView(new ArrayBuffer(8));
  view.setBigUint64(0, value, true);
  return view.getFloat64(0, true);
}
function next(value, up) {
  if (Number.isNaN(value) || value === (up ? Infinity : -Infinity)) return value;
  if (value === 0) return up ? Number.MIN_VALUE : -Number.MIN_VALUE;
  const valueBits = bits(value);
  const increase = (value > 0) === up;
  return fromBits(increase ? valueBits + 1n : valueBits - 1n);
}
function reference(x) {
  if (x < -0.25) {
    const square = x * x;
    return square + 0.5;
  }
  if (x < 0.0) {
    const scaled = x * 0.75;
    return scaled - 0.125;
  }
  if (x < 0.25) {
    const square = x * x;
    const cube = square * x;
    return cube + 0.25;
  }
  const shifted = x - 0.25;
  return shifted * 1.5;
}
function verify(instance) {
  const memory = instance.exports.memory;
  const piecewise = instance.exports.piecewise;
  const input = 1024;
  const output = 8192;
  const values = [
    next(-0.25, false), -0.25, next(-0.25, true),
    -0.0, 0.0, -Number.MIN_VALUE, Number.MIN_VALUE,
    next(0.25, false), 0.25, next(0.25, true),
    -Infinity, Infinity, fromBits(0x7ff8000000001234n),
  ];
  const view = new DataView(memory.buffer);
  values.forEach((value, index) => view.setFloat64(input + index * 8, value, true));
  piecewise(input, values.length, output, values.length);
  for (let index = 0; index < values.length; index += 1) {
    const actual = view.getFloat64(output + index * 8, true);
    const expected = reference(values[index]);
    if (Number.isNaN(expected) && Number.isNaN(actual)) continue;
    if (bits(actual) !== bits(expected)) {
      throw new Error("strict result at " + index + ": " + actual + " (" + bits(actual) + ") != " + expected + " (" + bits(expected) + ")");
    }
  }

  // Compare NaN sign and payload through raw memory bits. JS Number round trips
  // can canonicalize signaling NaNs, so these inputs never pass through Number.
  const payloads = [
    0x7ff8000000001234n,
    0xfff8000000004321n,
    0x7ff0000000001234n,
    0xfff0000000004321n,
  ];
  const payloadInput = 24576;
  const payloadOutput = 28672;
  payloads.forEach((value, index) => view.setBigUint64(payloadInput + index * 8, value, true));
  piecewise(payloadInput, payloads.length, payloadOutput, payloads.length);
  const payloadResults = payloads.map((_, index) => view.getBigUint64(payloadOutput + index * 8, true));

  const overlap = 16384;
  values.forEach((value, index) => view.setFloat64(overlap + index * 8, value, true));
  piecewise(overlap, values.length, overlap, values.length);
  for (let index = 0; index < values.length; index += 1) {
    const actual = view.getFloat64(overlap + index * 8, true);
    const expected = reference(values[index]);
    if (Number.isNaN(expected) && Number.isNaN(actual)) continue;
    if (bits(actual) !== bits(expected)) {
      throw new Error("overlap result at " + index + ": " + actual + " (" + bits(actual) + ") != " + expected + " (" + bits(expected) + ")");
    }
  }
  return payloadResults;
}
function verifyShiftedOverlap(actual, baseline) {
  const input = 32768;
  const length = 7;
  const values = [-0.5, -0.25, -0.0, 0.0, 0.25, 0.5, 0.125];
  const actualView = new DataView(actual.exports.memory.buffer);
  const baselineView = new DataView(baseline.exports.memory.buffer);
  values.forEach((value, index) => {
    actualView.setFloat64(input + index * 8, value, true);
    baselineView.setFloat64(input + index * 8, value, true);
  });
  actual.exports.piecewise(input, length, input + 8, length);
  baseline.exports.piecewise(input, length, input + 8, length);
  for (let index = 0; index < length + 1; index += 1) {
    const actualBits = actualView.getBigUint64(input + index * 8, true);
    const baselineBits = baselineView.getBigUint64(input + index * 8, true);
    if (actualBits !== baselineBits) {
      throw new Error("shifted overlapping input/output at " + index + ": " + actualBits + " != " + baselineBits);
    }
  }
}
WebAssembly.instantiate(bytes).then(async ({instance}) => {
  const actualPayloads = verify(instance);
  if (process.argv[3]) {
    const baselineBytes = fs.readFileSync(process.argv[3]);
    const {instance: baseline} = await WebAssembly.instantiate(baselineBytes);
    const baselinePayloads = verify(baseline);
    for (let index = 0; index < actualPayloads.length; index += 1) {
      if (actualPayloads[index] !== baselinePayloads[index]) {
        throw new Error("NaN payload/sign at " + index + ": " + actualPayloads[index] + " != " + baselinePayloads[index]);
      }
    }
    verifyShiftedOverlap(instance, baseline);
  }
})
  .catch(error => { console.error(error); process.exitCode = 1; });
"#;
    if node_available() {
        assert!(run_node(&wasm, Some(&baseline), runner).is_some());
        assert!(run_node(&wat_bytes, Some(&baseline), runner).is_some());
    }
}

#[test]
fn wasm_piecewise_select_should_keep_the_completed_write_prefix_on_oob_trap() {
    let (wat, wasm) = emitted_piecewise(PIECEWISE_SOURCE, 3);
    let wat_bytes = wat::parse_str(&wat).expect("piecewise WAT parses");
    let runner = r#"
const fs = require("node:fs");
const bytes = fs.readFileSync(process.argv[2]);
const expected = [-0.25 * 0.75 - 0.125, (0.5 - 0.25) * 1.5];
WebAssembly.instantiate(bytes).then(({instance}) => {
  const memory = instance.exports.memory;
  const piecewise = instance.exports.piecewise;
  const input = 1024;
  const view = new DataView(memory.buffer);
  [-0.25, 0.5, 0.0, 0.25].forEach((value, index) => view.setFloat64(input + index * 8, value, true));
  const output = memory.buffer.byteLength - 16;
  let trapped = false;
  try {
    piecewise(input, 4, output, 4);
  } catch (error) {
    trapped = /out of bounds|memory access/.test(String(error));
  }
  if (!trapped) throw new Error("expected the third output store to trap at linear-memory end");
  for (let index = 0; index < expected.length; index += 1) {
    const actual = view.getFloat64(output + index * 8, true);
    if (!Object.is(actual, expected[index])) {
      throw new Error("write prefix " + index + ": " + actual + " != " + expected[index]);
    }
  }
}).catch(error => { console.error(error); process.exitCode = 1; });
"#;
    if node_available() {
        assert!(run_node(&wasm, None, runner).is_some());
        assert!(run_node(&wat_bytes, None, runner).is_some());
    }
}

#[test]
fn wasm_piecewise_select_should_fall_back_for_different_store_addresses_or_division_tests() {
    let different_index = PIECEWISE_SOURCE.replace(
        "out[i] = x * 0.75 - 0.125;",
        "out[i + 1] = x * 0.75 - 0.125;",
    );
    let division_condition = PIECEWISE_SOURCE.replace("if x < -0.25 {", "if x / 2.0 < -0.25 {");
    let conditional_load = PIECEWISE_SOURCE.replace("if x < 0.0 {", "if input[i + 1] < 0.0 {");
    let non_unit_update =
        PIECEWISE_SOURCE.replace("i = i + 1;\n      continue;", "i = i + 2;\n      continue;");
    for source in [
        different_index,
        division_condition,
        conditional_load,
        non_unit_update,
    ] {
        let (wat, _) = emitted_piecewise(&source, 3);
        let body = piecewise_body(&wat);
        assert!(
            body.matches("f64.store").count() > 1,
            "unsafe tree shape must retain its separate stores:\n{body}"
        );
        assert!(
            body.contains("if\n"),
            "unsafe tree shape must retain control flow:\n{body}"
        );
    }
}
