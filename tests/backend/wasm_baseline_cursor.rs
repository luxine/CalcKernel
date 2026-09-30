use calckernel::{
    BoundsMode, EmitWasmOptions, KirConsumer, OverflowMode, emit_wasm_kir_module,
    emit_wat_kir_module,
};

use super::support::command::node_available;
use super::support::compiler::{optimized_module, verified_artifact};
use std::{fs, process::Command};

const SUM_F64: &str = r#"
export fn sum_f64(values: slice<f64>, start: u32, end: u32) -> f64 {
  let i: u32 = start;
  let total: f64 = 0.0;
  while i < end {
    total = total + values[i];
    i = i + 1;
  }
  return total;
}
"#;

const COPY_OFFSET_U32: &str = r#"
export fn copy_offset_u32(
  dst: slice<u32>, src: slice<u32>, start: u32, end: u32, offset: u32
) -> void {
  let i: u32 = start;
  while i < end {
    dst[i + offset] = src[i + offset];
    i = i + 1;
  }
}
"#;

const COPY_PREVIOUS_U32: &str = r#"
export fn copy_previous_u32(dst: slice<u32>, src: slice<u32>, start: u32, end: u32) -> void {
  let i: u32 = start;
  while i < end {
    dst[i - 1] = src[i - 1];
    i = i + 1;
  }
}
"#;

const COPY_DYNAMIC_PREVIOUS_U32: &str = r#"
export fn copy_dynamic_previous_u32(
  dst: slice<u32>, src: slice<u32>, start: u32, end: u32, offset: u32
) -> void {
  let i: u32 = start;
  while i < end {
    dst[i - offset] = src[i - offset];
    i = i + 1;
  }
}
"#;

const AFFINE_NEIGHBOR_COPY_F64: &str = r#"
export fn affine_neighbor_copy_f64(
  dst: slice<f64>, src: slice<f64>, start: u32, end: u32, row_base: u32, width: u32
) -> void {
  let i: u32 = start;
  while i < end {
    dst[row_base + i - width - 1] = src[row_base + i - width - 1];
    dst[row_base + i - width] = src[row_base + i - width];
    dst[row_base + i - width + 1] = src[row_base + i - width + 1];
    i = i + 1;
  }
}
"#;

fn emit(source: &str, opt_level: u8, bounds_mode: BoundsMode) -> (String, Vec<u8>) {
    let optimized = optimized_module(
        source,
        opt_level,
        KirConsumer::WebAssembly,
        OverflowMode::Unchecked,
        bounds_mode,
    );
    let artifact = verified_artifact(&optimized);
    let options = EmitWasmOptions { opt_level };
    (
        emit_wat_kir_module(artifact, options).expect("verified KIR emits WAT"),
        emit_wasm_kir_module(artifact, options).expect("verified KIR emits binary Wasm"),
    )
}

fn run_node(bytes: &[u8], runner: &str) -> Option<String> {
    if !node_available() {
        return None;
    }
    let directory = super::support::temp::temp_dir("ck-wasm-baseline-cursor");
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
fn wasm_o3_routes_scalar_four_block_reduction_through_typed_loop_rotation() {
    let (o0_wat, o0_wasm) = emit(SUM_F64, 0, BoundsMode::Unchecked);
    let (o3_wat, o3_wasm) = emit(SUM_F64, 3, BoundsMode::Unchecked);
    let o0_body = o0_wat
        .split("(func $sum_f64")
        .nth(1)
        .expect("sum function body");
    let o3_body = o3_wat
        .split("(func $sum_f64")
        .nth(1)
        .expect("sum function body");
    assert!(
        o0_body.contains("br $ik_dispatch"),
        "O0 retains dispatcher:\n{o0_wat}"
    );
    assert!(
        o3_body.contains("br_if $ik_loop_"),
        "O3 uses the validated typed rotated loop:\n{o3_wat}"
    );
    assert_eq!(
        o3_body.matches("f64.add").count(),
        1,
        "strict sum still adds once per item"
    );

    let wat_binary = wat::parse_str(&o3_wat).expect("O3 WAT parses to a module");
    let runner = r#"
const fs = require("node:fs");
WebAssembly.instantiate(fs.readFileSync(process.argv[2])).then(({instance}) => {
  const view = new DataView(instance.exports.memory.buffer);
  [1.25, -2.5, 8.0, 0.125].forEach((value, index) => view.setFloat64(64 + index * 8, value, true));
  const actual = instance.exports.sum_f64(64, 4, 1, 4);
  if (actual !== 5.625) throw new Error(`sum result ${actual}`);
  process.stdout.write(String(actual));
}).catch(error => { console.error(error); process.exitCode = 1; });
"#;
    let Some(o0_actual) = run_node(&o0_wasm, runner) else {
        return;
    };
    assert_eq!(o0_actual, "5.625");
    assert_eq!(run_node(&o3_wasm, runner).as_deref(), Some("5.625"));
    assert_eq!(run_node(&wat_binary, runner).as_deref(), Some("5.625"));
}

#[test]
fn wasm_o0_o3_slice_offset_cursor_preserves_values_and_physical_end() {
    let (o0_wat, o0_wasm) = emit(COPY_OFFSET_U32, 0, BoundsMode::Unchecked);
    let (o3_wat, o3_wasm) = emit(COPY_OFFSET_U32, 3, BoundsMode::Unchecked);
    let o3_body = o3_wat
        .split("(func $copy_offset_u32")
        .nth(1)
        .expect("copy function body");
    assert!(
        o3_body.contains("local $ik_mem_cursor"),
        "O3 owns checked cursors:\n{o3_wat}"
    );
    assert_eq!(
        o3_body.matches("local.get $ik_mem_cursor").count(),
        4,
        "the two indexed accesses and two backedge advances use their loop-carried cursors"
    );
    assert!(
        o0_wat.contains("i32.mul"),
        "O0 retains indexed addressing:\n{o0_wat}"
    );

    let runner = r#"
const fs = require("node:fs");
WebAssembly.instantiate(fs.readFileSync(process.argv[2])).then(({instance}) => {
  const words = new Uint32Array(instance.exports.memory.buffer);
  const src = 64, dst = 128;
  [11, 22, 33, 44, 55].forEach((value, i) => words[src / 4 + i] = value);
  words.fill(0xdeadbeef, dst / 4, dst / 4 + 7);
  instance.exports.copy_offset_u32(dst, 5, src, 5, 0, 3, 1);
  const actual = Array.from(words.slice(dst / 4, dst / 4 + 7));
const expected = [0xdeadbeef, 22, 33, 44, 0xdeadbeef, 0xdeadbeef, 0xdeadbeef];
  if (actual.some((value, i) => value !== expected[i])) throw new Error(`copy ${actual}`);
  words[0] = 0x1234abcd;
  words.fill(0xdeadbeef, dst / 4, dst / 4 + 7);
  instance.exports.copy_offset_u32(dst, 5, 0xfffffffc, 2, 0, 1, 1);
  if (words[dst / 4 + 1] !== 0x1234abcd) throw new Error("dynamic offset did not wrap Wasm32 address");
  process.stdout.write(actual.join(","));
}).catch(error => { console.error(error); process.exitCode = 1; });
"#;
    let Some(o0_actual) = run_node(&o0_wasm, runner) else {
        return;
    };
    let o3_actual = run_node(&o3_wasm, runner).expect("Node available for O3 module");
    assert_eq!(o3_actual, o0_actual);
    let wat_binary = wat::parse_str(&o3_wat).expect("O3 WAT parses to a module");
    assert_eq!(
        run_node(&wat_binary, runner).as_deref(),
        Some(o0_actual.as_str())
    );
}

#[test]
fn wasm_slice_cursor_constant_negative_offset_keeps_wasm32_wraparound() {
    let (_o0_wat, o0_wasm) = emit(COPY_PREVIOUS_U32, 0, BoundsMode::Unchecked);
    let (o3_wat, o3_wasm) = emit(COPY_PREVIOUS_U32, 3, BoundsMode::Unchecked);
    let o3_body = o3_wat
        .split("(func $copy_previous_u32")
        .nth(1)
        .expect("copy function body");
    assert!(
        o3_body.contains("local $ik_mem_cursor"),
        "O3 uses a slice cursor:\n{o3_wat}"
    );
    assert_eq!(o3_body.matches("local.get $ik_mem_cursor").count(), 4);

    let runner = r#"
const fs = require("node:fs");
WebAssembly.instantiate(fs.readFileSync(process.argv[2])).then(({instance}) => {
  const words = new Uint32Array(instance.exports.memory.buffer);
  words[0] = 0x1234abcd;
  words.fill(0xdeadbeef, 31, 34);
  instance.exports.copy_previous_u32(128, 3, 4, 3, 0, 1);
  if (words[31] !== 0x1234abcd || words[32] !== 0xdeadbeef || words[33] !== 0xdeadbeef) {
    throw new Error(`wrapped output ${Array.from(words.slice(31, 34))}`);
  }
  process.stdout.write(String(words[31]));
}).catch(error => { console.error(error); process.exitCode = 1; });
"#;
    let Some(o0_actual) = run_node(&o0_wasm, runner) else {
        return;
    };
    assert_eq!(o0_actual, "305441741");
    assert_eq!(
        run_node(&o3_wasm, runner).as_deref(),
        Some(o0_actual.as_str())
    );
    let wat_binary = wat::parse_str(&o3_wat).expect("O3 WAT parses to a module");
    assert_eq!(
        run_node(&wat_binary, runner).as_deref(),
        Some(o0_actual.as_str())
    );
}

#[test]
fn wasm_slice_cursor_dynamic_negative_offset_keeps_wasm32_wraparound() {
    let (_o0_wat, o0_wasm) = emit(COPY_DYNAMIC_PREVIOUS_U32, 0, BoundsMode::Unchecked);
    let (o3_wat, o3_wasm) = emit(COPY_DYNAMIC_PREVIOUS_U32, 3, BoundsMode::Unchecked);
    let o3_body = o3_wat
        .split("(func $copy_dynamic_previous_u32")
        .nth(1)
        .expect("copy function body");
    assert!(
        o3_body.contains("local $ik_mem_cursor"),
        "O3 uses a slice cursor:\n{o3_wat}"
    );
    assert_eq!(o3_body.matches("local.get $ik_mem_cursor").count(), 4);

    let runner = r#"
const fs = require("node:fs");
WebAssembly.instantiate(fs.readFileSync(process.argv[2])).then(({instance}) => {
  const words = new Uint32Array(instance.exports.memory.buffer);
  words[0] = 0x76543210;
  words.fill(0xdeadbeef, 31, 34);
  instance.exports.copy_dynamic_previous_u32(128, 3, 4, 3, 0, 1, 1);
  if (words[31] !== 0x76543210 || words[32] !== 0xdeadbeef || words[33] !== 0xdeadbeef) {
    throw new Error(`wrapped output ${Array.from(words.slice(31, 34))}`);
  }
  process.stdout.write(String(words[31]));
}).catch(error => { console.error(error); process.exitCode = 1; });
"#;
    let Some(o0_actual) = run_node(&o0_wasm, runner) else {
        return;
    };
    assert_eq!(o0_actual, "1985229328");
    assert_eq!(
        run_node(&o3_wasm, runner).as_deref(),
        Some(o0_actual.as_str())
    );
    let wat_binary = wat::parse_str(&o3_wat).expect("O3 WAT parses to a module");
    assert_eq!(
        run_node(&wat_binary, runner).as_deref(),
        Some(o0_actual.as_str())
    );
}

#[test]
fn wasm_slice_cursor_keeps_physical_trap_order_and_written_prefix() {
    let (_o0_wat, o0_wasm) = emit(COPY_OFFSET_U32, 0, BoundsMode::Unchecked);
    let (o3_wat, o3_wasm) = emit(COPY_OFFSET_U32, 3, BoundsMode::Unchecked);
    let o3_body = o3_wat
        .split("(func $copy_offset_u32")
        .nth(1)
        .expect("copy function body");
    assert!(
        o3_body.contains("local $ik_mem_cursor"),
        "O3 uses checked cursors:\n{o3_wat}"
    );
    assert_eq!(o3_body.matches("local.get $ik_mem_cursor").count(), 4);

    let runner = r#"
const fs = require("node:fs");
WebAssembly.instantiate(fs.readFileSync(process.argv[2])).then(({instance}) => {
  const words = new Uint32Array(instance.exports.memory.buffer);
  const dst = 128;
  words[16383] = 42;
  words.fill(0xdeadbeef, dst / 4, dst / 4 + 6);
  let trapped = false;
  try { instance.exports.copy_offset_u32(dst, 6, 65532, 4, 0, 3, 0); }
  catch (error) { if (!(error instanceof WebAssembly.RuntimeError)) throw error; trapped = true; }
  if (!trapped) throw new Error("expected physical WebAssembly memory trap");
  const actual = Array.from(words.slice(dst / 4, dst / 4 + 6));
  const expected = [42, 0xdeadbeef, 0xdeadbeef, 0xdeadbeef, 0xdeadbeef, 0xdeadbeef];
  if (actual.some((value, i) => value !== expected[i])) throw new Error(`trap prefix ${actual}`);
  process.stdout.write(actual.join(","));
}).catch(error => { console.error(error); process.exitCode = 1; });
"#;
    let Some(o0_actual) = run_node(&o0_wasm, runner) else {
        return;
    };
    assert_eq!(
        run_node(&o3_wasm, runner).as_deref(),
        Some(o0_actual.as_str()),
        "O3 cursor path preserves O0 trap prefix"
    );
    let wat_binary = wat::parse_str(&o3_wat).expect("checked O3 WAT parses to a module");
    assert_eq!(
        run_node(&wat_binary, runner).as_deref(),
        Some(o0_actual.as_str())
    );
}

#[test]
fn wasm_o0_o3_affine_cursor_groups_neighbor_addresses_and_preserves_memory_semantics() {
    let (o0_wat, o0_wasm) = emit(AFFINE_NEIGHBOR_COPY_F64, 0, BoundsMode::Unchecked);
    let (o3_wat, o3_wasm) = emit(AFFINE_NEIGHBOR_COPY_F64, 3, BoundsMode::Unchecked);
    let o3_body = o3_wat
        .split("(func $affine_neighbor_copy_f64")
        .nth(1)
        .expect("affine copy function body");
    assert_eq!(
        o3_body.matches("(local $ik_mem_cursor").count(),
        2,
        "the source and destination share affine neighbor cursors:\n{o3_wat}"
    );
    let o3_lines = o3_body.lines().map(str::trim).collect::<Vec<_>>();
    assert!(
        o3_lines.windows(2).any(|pair| {
            matches!(pair[0], "i32.const 4294967288" | "i32.const -8") && pair[1] == "i32.add"
        }),
        "negative neighbor byte bias is applied with wrapping i32.add:\n{o3_wat}"
    );

    let runner = r#"
const fs = require("node:fs");
WebAssembly.instantiate(fs.readFileSync(process.argv[2])).then(({instance}) => {
  const bytes = new Uint8Array(instance.exports.memory.buffer);
  const view = new DataView(instance.exports.memory.buffer);
  const src = 64, dst = 256;
  const sentinel = 0x7ff8000000000042n;
  const patterns = [0x8000000000000000n, 0n, sentinel, 0x3ff0000000000000n,
                    0xbff0000000000000n, 0x0000000000000001n, 0x400921fb54442d18n];
  const snapshots = [];
  for (const [end, row, width] of [[0, 2, 1], [1, 2, 1], [2, 2, 1],
                                   [3, 2, 1], [5, 2, 1], [4, 4, 3]]) {
    bytes.fill(0xa5, 0, 1024);
    for (let i = 0; i < 16; i++) view.setBigUint64(src + i * 8, patterns[i % patterns.length], true);
    instance.exports.affine_neighbor_copy_f64(dst, 16, src, 16, 0, end, row, width);
    snapshots.push(bytes.slice(src, src + 128), bytes.slice(dst, dst + 96));
  }

  // The three ranges overlap intentionally. Forward source order must be retained.
  bytes.fill(0xa5, 0, 1024);
  for (let i = 0; i < 12; i++) view.setBigUint64(src + i * 8, patterns[i % patterns.length], true);
  instance.exports.affine_neighbor_copy_f64(src, 16, src, 16, 0, 4, 2, 1);
  const alias = bytes.slice(src, src + 96);

  // Modular u32 index and byte-address arithmetic wrap back into the first page.
  bytes.fill(0xa5, 0, 1024);
  for (const offset of [56, 64, 72]) {
    view.setBigUint64(src + offset, patterns[(offset / 8) % patterns.length], true);
  }
  instance.exports.affine_neighbor_copy_f64(dst, 16, src, 16, 0, 1, 0x20000000, 0);
  const wrapped = bytes.slice(dst + 48, dst + 88);

  // The first source read succeeds and its destination store commits; the next
  // source read physically traps. The optimized loop must expose the same prefix.
  bytes.fill(0xa5, 0, 1024);
  bytes.fill(0x5a, 512, 560);
  view.setFloat64(65528, 19.25, true);
  let trapped = false;
  try { instance.exports.affine_neighbor_copy_f64(512, 8, 65528, 8, 0, 4, 2, 1); }
  catch (error) { if (!(error instanceof WebAssembly.RuntimeError)) throw error; trapped = true; }
  if (!trapped) throw new Error("expected physical memory trap");
  const prefix = bytes.slice(512, 560);
  process.stdout.write(Buffer.from([...snapshots.flatMap(value => [...value]),
                                    ...alias, ...wrapped, ...prefix]).toString("hex"));
}).catch(error => { console.error(error); process.exitCode = 1; });
"#;
    let Some(o0_actual) = run_node(&o0_wasm, runner) else {
        return;
    };
    assert_eq!(
        run_node(&o3_wasm, runner).as_deref(),
        Some(o0_actual.as_str())
    );
    let wat_binary = wat::parse_str(&o3_wat).expect("O3 WAT parses to a module");
    assert_eq!(
        run_node(&wat_binary, runner).as_deref(),
        Some(o0_actual.as_str())
    );
    assert!(o0_wat.contains("i32.mul"), "O0 retains indexed addressing");
}
