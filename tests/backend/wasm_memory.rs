use calckernel::{
    BoundsMode, EmitWasmOptions, KirConsumer, KirTerminator, OverflowMode, emit_wasm_kir_module,
    emit_wat_kir_module,
};

use super::support::command::node_available;
use super::support::compiler::{optimized_module, verified_artifact};
use std::{
    fs,
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT_WASM_MEMORY_ID: AtomicU64 = AtomicU64::new(0);

const COPY_U32: &str = r#"
export unsafe fn copy_u32(dst: ptr<u32>, src: ptr<u32>, start: u32, end: u32) -> void contract {
  requires start <= end;
} {
  let i: u32 = start;
  while i < end {
    dst[i] = src[i];
    i = i + 1;
  }
}

export unsafe fn copy_skip(dst: ptr<u32>, src: ptr<u32>, start: u32, end: u32, skip: u32) -> void contract {
  requires start <= end;
} {
  let i: u32 = start;
  while i < end {
    if i == skip {
      i = i + 1;
      continue;
    }
    dst[i] = src[i];
    i = i + 1;
  }
}
"#;

const FILL_U32: &str = r#"
export unsafe fn fill_u32(dst: ptr<u32>, start: u32, end: u32) -> void contract {
  requires start <= end;
} {
  let i: u32 = start;
  while i < end {
    dst[i] = 1515870810;
    i = i + 1;
  }
}
"#;

const COPY_I32: &str = r#"
export unsafe fn copy_i32(dst: ptr<i32>, src: ptr<i32>, start: u32, end: u32) -> void contract {
  requires start <= end;
} {
  let i: u32 = start;
  while i < end {
    dst[i] = src[i];
    i = i + 1;
  }
}
"#;

const FILL_U32_AB: &str = r#"
export unsafe fn fill_u32_ab(dst: ptr<u32>, start: u32, end: u32) -> void contract {
  requires start <= end;
} {
  let i: u32 = start;
  while i < end {
    dst[i] = 2880154539;
    i = i + 1;
  }
}
"#;

const FILL_U32_NONREPEATED_WORD: &str = r#"
export unsafe fn fill_u32_nonrepeated(dst: ptr<u32>, start: u32, end: u32) -> void contract {
  requires start <= end;
} {
  let i: u32 = start;
  while i < end {
    dst[i] = 1;
    i = i + 1;
  }
}
"#;

const SLOT_OFFSETS: &str = include_str!("../../examples/wasm/field_offset.ck");

const COPY_SWITCHED_BASE: &str = r#"
export unsafe fn copy_switch(dst: ptr<u32>, src: ptr<u32>, alternate: ptr<u32>, start: u32, end: u32) -> void contract {
  requires start <= end;
} {
  let i: u32 = start;
  while i < end {
    dst[i] = src[i];
    i = i + 1;
  }
}
"#;

const COPY_F64: &str = r#"
export unsafe fn copy_f64(dst: ptr<f64>, src: ptr<f64>, start: u32, end: u32) -> void contract {
  requires start <= end;
} {
  let i: u32 = start;
  while i < end {
    dst[i] = src[i];
    i = i + 1;
  }
}
"#;

const COPY_SLOT_FIELD: &str = r#"
struct Slot {
  head: u32;
  value: u32;
  tail: u64;
}
export unsafe fn copy_slot_value(dst: ptr<u32>, src: ptr<Slot>, start: u32, end: u32) -> void contract {
  requires start <= end;
} {
  let i: u32 = start;
  while i < end {
    dst[i] = src[i].value;
    i = i + 1;
  }
}
"#;

const NESTED_FIELD: &str = r#"
struct Inner {
  marker: u32;
  value: u32;
}
struct Record {
  head: u32;
  inner: Inner;
  tail: u64;
}
export unsafe fn load_nested(items: ptr<Record>, index: u32) -> u32 contract {
  requires index >= 0;
} {
  return items[index].inner.value;
}
"#;

const SLOT_WITH_UNUSED_SLICE: &str = r#"
struct Slot {
  head: u32;
  value: u32;
  tail: u64;
}
export unsafe fn load_with_slice_spare(items: ptr<Slot>, spare: slice<u32>, index: u32) -> u32 contract {
  requires aligned(items, 16);
} {
  return items[index].value;
}
"#;

const SLOT_WITH_POINTER_SLICE_SPARE: &str = r#"
struct Slot {
  head: u32;
  value: u32;
  tail: u64;
}
export unsafe fn load_with_pointer_spare(items: ptr<Slot>, spare: ptr<slice<u32>>, index: u32) -> u32 contract {
  requires aligned(items, 16);
} {
  return items[index].value;
}
"#;

fn emit_copy(opt_level: u8) -> (String, Vec<u8>) {
    emit_source(COPY_U32, opt_level)
}

fn emit_source(source: &str, opt_level: u8) -> (String, Vec<u8>) {
    let optimized = optimize_source(source, opt_level);
    let module = verified_artifact(&optimized);
    let options = EmitWasmOptions { opt_level };
    (
        emit_wat_kir_module(module, options).expect("verified copy KIR should emit WAT"),
        emit_wasm_kir_module(module, options).expect("verified copy KIR should emit Wasm"),
    )
}

fn emit_wat_through_ckc(source: &str, opt_level: u8, features: &str) -> String {
    let id = NEXT_WASM_MEMORY_ID.fetch_add(1, Ordering::Relaxed);
    let temp_root = std::env::temp_dir().join(format!(
        "calckernel_wasm_memory_cli_{}_{}",
        std::process::id(),
        id
    ));
    fs::create_dir(&temp_root).expect("create temporary CLI fixture directory");
    let source_path = temp_root.join("field_offset.ck");
    let wat_path = temp_root.join("field_offset.wat");
    fs::write(&source_path, source).expect("write CK source");
    let output = Command::new(env!("CARGO_BIN_EXE_ckc"))
        .arg("emit-wat")
        .arg(&source_path)
        .arg("--out")
        .arg(&wat_path)
        .arg("--overflow")
        .arg("unchecked")
        .arg("--bounds")
        .arg("unchecked")
        .arg("--wasm-features")
        .arg(features)
        .arg("--opt-level")
        .arg(opt_level.to_string())
        .output()
        .expect("run ckc emit-wat");
    assert!(
        output.status.success(),
        "ckc emit-wat failed with {:?}:\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
    let wat = fs::read_to_string(&wat_path).expect("read emitted WAT");
    let _ = fs::remove_dir_all(temp_root);
    wat
}

fn emit_wasm_through_ckc(source: &str, opt_level: u8, features: &str) -> Vec<u8> {
    let id = NEXT_WASM_MEMORY_ID.fetch_add(1, Ordering::Relaxed);
    let temp_root = std::env::temp_dir().join(format!(
        "calckernel_wasm_memory_cli_{}_{}",
        std::process::id(),
        id
    ));
    fs::create_dir(&temp_root).expect("create temporary CLI fixture directory");
    let source_path = temp_root.join("field_offset.ck");
    let wasm_path = temp_root.join("field_offset.wasm");
    fs::write(&source_path, source).expect("write CK source");
    let output = Command::new(env!("CARGO_BIN_EXE_ckc"))
        .arg("emit-wasm")
        .arg(&source_path)
        .arg("--out")
        .arg(&wasm_path)
        .arg("--overflow")
        .arg("unchecked")
        .arg("--bounds")
        .arg("unchecked")
        .arg("--wasm-features")
        .arg(features)
        .arg("--opt-level")
        .arg(opt_level.to_string())
        .output()
        .expect("run ckc emit-wasm");
    assert!(
        output.status.success(),
        "ckc emit-wasm failed with {:?}:\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
    let wasm = fs::read(&wasm_path).expect("read emitted Wasm");
    let _ = fs::remove_dir_all(temp_root);
    wasm
}

fn optimize_source(source: &str, opt_level: u8) -> calckernel::KirPassManagerResult {
    optimized_module(
        source,
        opt_level,
        KirConsumer::WebAssembly,
        OverflowMode::Unchecked,
        BoundsMode::Unchecked,
    )
}

fn mutate_copy_switch_backedge(opt_level: u8) -> calckernel::KirModule {
    let optimized = optimize_source(COPY_SWITCHED_BASE, opt_level);
    let mut module = verified_artifact(&optimized).clone();
    let function = module
        .functions
        .iter_mut()
        .find(|function| function.name == "copy_switch")
        .expect("copy_switch function");
    let alternate = function
        .params
        .iter()
        .find(|param| param.name == "alternate")
        .expect("alternate pointer parameter")
        .value;
    let loop_header = function
        .blocks
        .iter()
        .find(|block| {
            matches!(block.terminator, KirTerminator::Branch { .. })
                && block.params.iter().any(|param| param.slot == "i")
                && block.params.iter().any(|param| param.slot == "src")
        })
        .expect("canonical loop header");
    let src_arg_index = loop_header
        .params
        .iter()
        .position(|param| param.slot == "src")
        .expect("loop-carried source parameter");
    let loop_header_id = loop_header.id;
    let mut changed_backedges = 0;
    for block in &mut function.blocks {
        let carries_store = block.instructions.iter().any(|instruction| {
            matches!(
                instruction.kind,
                calckernel::KirInstructionKind::Store { .. }
            )
        });
        if !carries_store {
            continue;
        }
        if let KirTerminator::Jump { edge } = &mut block.terminator
            && edge.target == loop_header_id
        {
            edge.args[src_arg_index] = alternate;
            changed_backedges += 1;
        }
    }
    assert_eq!(changed_backedges, 1, "mutate exactly one loop backedge");
    assert_eq!(calckernel::validate_kir_module(&module).errors, []);
    module
}

fn emit_mutated_copy_switch(opt_level: u8) -> (String, Vec<u8>) {
    let module = mutate_copy_switch_backedge(opt_level);
    let options = EmitWasmOptions { opt_level };
    (
        emit_wat_kir_module(&module, options).expect("mutated KIR should fail closed or lower"),
        emit_wasm_kir_module(&module, options).expect("mutated KIR should fail closed or lower"),
    )
}

#[test]
fn wasm_o3_should_advance_a_checked_memory_cursor_instead_of_recomputing_indexed_addresses() {
    let (o0_wat, _) = emit_copy(0);
    let (o3_wat, _) = emit_copy(3);

    assert!(o0_wat.contains("loop $ik_dispatch"), "{o0_wat}");
    assert!(o3_wat.contains("loop $ik_loop"), "{o3_wat}");

    let o3_loop = o3_wat
        .split_once("loop $ik_loop")
        .expect("O3 structured loop")
        .1
        .split_once("br $ik_loop")
        .expect("O3 loop backedge")
        .0;
    assert_eq!(
        o3_loop.matches("i32.mul").count(),
        0,
        "O3 should use a loop-carried address cursor instead of scaling both addresses per iteration:\n{o3_loop}"
    );

    let o0_loop = o0_wat
        .split_once("loop $ik_dispatch")
        .expect("O0 dispatcher loop")
        .1
        .split_once("br $ik_dispatch")
        .expect("O0 dispatcher backedge")
        .0;
    assert_eq!(
        o0_loop.matches("i32.mul").count(),
        2,
        "the O0 reference should retain its two indexed address calculations:\n{o0_loop}"
    );
}

#[test]
fn wasm_o3_should_emit_bulk_memory_only_for_checked_copy_and_fill_loops() {
    let (o0_copy, _) = emit_source(COPY_U32, 0);
    let (o3_copy, _) = emit_source(COPY_U32, 3);
    let (o3_fill, _) = emit_source(FILL_U32, 3);
    let (o3_i32_copy, _) = emit_source(COPY_I32, 3);
    let (o3_ab_fill, _) = emit_source(FILL_U32_AB, 3);
    let (o3_nonrepeated_fill, _) = emit_source(FILL_U32_NONREPEATED_WORD, 3);

    assert!(
        !o0_copy.contains("memory.copy"),
        "O0 must remain scalar:\n{o0_copy}"
    );
    assert!(
        o3_copy.contains("memory.copy"),
        "O3 should emit a checked copy fast path:\n{o3_copy}"
    );
    assert!(
        o3_copy.contains("memory.size"),
        "the fast path must guard against the current memory size:\n{o3_copy}"
    );
    assert!(
        o3_fill.contains("memory.fill"),
        "O3 should emit a checked repeated-byte fill fast path:\n{o3_fill}"
    );
    assert!(
        o3_i32_copy.contains("memory.copy"),
        "ptr<i32> should share the checked copy subset:\n{o3_i32_copy}"
    );
    assert!(
        o3_ab_fill.contains("i32.const 171\n"),
        "the repeated 0xab byte must be selected from 0xabababab:\n{o3_ab_fill}"
    );
    assert!(
        !o3_nonrepeated_fill.contains("memory.fill"),
        "a non-repeated u32 word must keep the scalar loop:\n{o3_nonrepeated_fill}"
    );

    let copy_skip = o3_copy
        .split_once("(func $copy_skip")
        .expect("multi-exit loop follows the eligible copy")
        .1;
    assert!(
        !copy_skip.contains("memory.copy"),
        "a loop with a conditional continue must remain scalar:\n{copy_skip}"
    );
    let (mutated, _) = emit_mutated_copy_switch(3);
    assert!(
        !mutated.contains("memory.copy"),
        "a changed source pointer on the loop backedge must fail the independent checker:\n{mutated}"
    );
}

#[test]
fn wasm_bulk_copy_should_preserve_scalar_overlap_wrap_and_partial_trap_behavior() {
    if !node_available() {
        return;
    }

    const RUNNER: &str = r#"
const fs = require("node:fs");
WebAssembly.instantiate(fs.readFileSync(process.argv[1]))
  .then(({ instance }) => {
    const { memory, copy_u32 } = instance.exports;
    const words = new Uint32Array(memory.buffer);

    words[65] = 0x12345678; words[66] = 0x23456789; words[67] = 0x3456789a;
    copy_u32(512, 256, 1, 4);
    if (words[129] !== words[65] || words[130] !== words[66] || words[131] !== words[67]) process.exit(1);

    words[20] = 1; words[21] = 2; words[22] = 3; words[23] = 4;
    copy_u32(84, 80, 0, 4);
    if (words[21] !== 1 || words[22] !== 1 || words[23] !== 1 || words[24] !== 1) process.exit(2);

    words[30] = 5; words[31] = 6; words[32] = 7; words[33] = 8;
    copy_u32(120, 124, 0, 3);
    if (words[30] !== 6 || words[31] !== 7 || words[32] !== 8) process.exit(3);

    words[16] = 0xaabbccdd;
    copy_u32(0xfffffff8, 0xfffffff8, 0xffffffff, 0xffffffff);
    if (words[16] !== 0xaabbccdd) process.exit(4);

    words[0] = 0x10293847;
    copy_u32(64, 0, 0x40000000, 0x40000001);
    if (words[16] !== 0x10293847) process.exit(5);

    words[0] = 0x55667788;
    copy_u32(0xfffffff4, 0xfffffff0, 4, 5);
    if (words[1] !== 0x55667788) process.exit(6);

    words[16383] = 0xcafebabe;
    let sourceTrap = false;
    try { copy_u32(0x2000, 65532, 0, 2); }
    catch (error) { sourceTrap = error instanceof WebAssembly.RuntimeError; }
    if (!sourceTrap || words[2048] !== 0xcafebabe) process.exit(7);

    words[2049] = 0x0badf00d;
    let destinationTrap = false;
    try { copy_u32(65532, 8192, 0, 2); }
    catch (error) { destinationTrap = error instanceof WebAssembly.RuntimeError; }
    if (!destinationTrap || words[16383] !== words[2048]) process.exit(8);
  })
  .catch((error) => { console.error(error); process.exit(9); });
"#;

    for opt_level in [0, 3] {
        let (_, wasm) = emit_source(COPY_U32, opt_level);
        let id = NEXT_WASM_MEMORY_ID.fetch_add(1, Ordering::Relaxed);
        let wasm_path = std::env::temp_dir().join(format!(
            "calckernel_wasm_bulk_copy_{}_o{opt_level}.wasm",
            id
        ));
        fs::write(&wasm_path, wasm).expect("write Wasm module");
        let output = Command::new("node")
            .arg("-e")
            .arg(RUNNER)
            .arg(&wasm_path)
            .output()
            .expect("run Node Wasm harness");
        let _ = fs::remove_file(&wasm_path);
        assert!(
            output.status.success(),
            "O{opt_level} Node runtime failed with {:?}:\n{}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn wasm_bulk_fill_should_preserve_zero_wrap_and_partial_trap_behavior() {
    if !node_available() {
        return;
    }

    const RUNNER: &str = r#"
const fs = require("node:fs");
WebAssembly.instantiate(fs.readFileSync(process.argv[1]))
  .then(({ instance }) => {
    const { memory, fill_u32 } = instance.exports;
    const words = new Uint32Array(memory.buffer);

    words.fill(0xdeadbeef, 40, 44);
    fill_u32(160, 0, 3);
    if (words[40] !== 0x5a5a5a5a || words[41] !== 0x5a5a5a5a || words[42] !== 0x5a5a5a5a) process.exit(1);

    words[50] = 0x12345678;
    fill_u32(0xfffffff8, 0xffffffff, 0xffffffff);
    if (words[50] !== 0x12345678) process.exit(2);

    words[1] = 0x10203040;
    fill_u32(0xfffffff4, 4, 5);
    if (words[1] !== 0x5a5a5a5a) process.exit(3);

    words[16383] = 0xa5a5a5a5;
    let trapped = false;
    try { fill_u32(65532, 0, 2); }
    catch (error) { trapped = error instanceof WebAssembly.RuntimeError; }
    if (!trapped || words[16383] !== 0x5a5a5a5a) process.exit(4);
  })
  .catch((error) => { console.error(error); process.exit(5); });
"#;

    for opt_level in [0, 3] {
        let (_, wasm) = emit_source(FILL_U32, opt_level);
        let id = NEXT_WASM_MEMORY_ID.fetch_add(1, Ordering::Relaxed);
        let wasm_path = std::env::temp_dir().join(format!(
            "calckernel_wasm_bulk_fill_{}_o{opt_level}.wasm",
            id
        ));
        fs::write(&wasm_path, wasm).expect("write Wasm module");
        let output = Command::new("node")
            .arg("-e")
            .arg(RUNNER)
            .arg(&wasm_path)
            .output()
            .expect("run Node Wasm harness");
        let _ = fs::remove_file(&wasm_path);
        assert!(
            output.status.success(),
            "O{opt_level} Node runtime failed with {:?}:\n{}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn wasm_memory_cursor_should_match_o0_for_zero_trip_and_wasm32_wrapping() {
    if !node_available() {
        return;
    }

    const RUNNER: &str = r#"
const fs = require("node:fs");
WebAssembly.instantiate(fs.readFileSync(process.argv[1]))
  .then(({ instance }) => {
    const { memory, copy_u32, copy_skip } = instance.exports;
    const words = new Uint32Array(memory.buffer);

    words.fill(0xdeadbeef, 0, 64);
    words[2] = 0x10203040;
    words[3] = 0x50607080;
    words[4] = 0x90abcdef;
    copy_u32(128, 0, 2, 5);
    if (words[34] !== 0x10203040 || words[35] !== 0x50607080 || words[36] !== 0x90abcdef) process.exit(1);

    words[50] = 0x13579bdf;
    copy_u32(160, 0, 10, 10);
    if (words[50] !== 0x13579bdf) process.exit(2);

    // index * sizeof(u32) wraps to zero in the Wasm32 address calculation.
    words[0] = 0x2468ace0;
    copy_u32(64, 0, 0x40000000, 0x40000001);
    if (words[16] !== 0x2468ace0) process.exit(3);

    // The base plus the scaled index wraps to address zero on Wasm32.
    words[0] = 0x0badcafe;
    copy_u32(0xfffffff4, 0xfffffff0, 4, 5);
    if (words[1] !== 0x0badcafe) process.exit(4);

    words[100] = 1; words[101] = 2; words[102] = 3; words[103] = 4;
    words.fill(0xfeedface, 200, 204);
    copy_skip(800, 400, 0, 4, 2);
    if (words[200] !== 1 || words[201] !== 2 || words[202] !== 0xfeedface || words[203] !== 4) process.exit(6);
  })
  .catch((error) => { console.error(error); process.exit(5); });
"#;

    for opt_level in [0, 3] {
        let (_, wasm) = emit_copy(opt_level);
        let id = NEXT_WASM_MEMORY_ID.fetch_add(1, Ordering::Relaxed);
        let wasm_path = std::env::temp_dir().join(format!(
            "calckernel_wasm_memory_cursor_{}_o{opt_level}.wasm",
            id
        ));
        fs::write(&wasm_path, wasm).expect("write Wasm module");
        let output = Command::new("node")
            .arg("-e")
            .arg(RUNNER)
            .arg(&wasm_path)
            .output()
            .expect("run Node Wasm harness");
        let _ = fs::remove_file(&wasm_path);
        assert!(
            output.status.success(),
            "O{opt_level} Node runtime failed with {:?}:\n{}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn wasm_o3_should_use_an_eight_byte_cursor_for_f64_copy() {
    let (o0_wat, _) = emit_source(COPY_F64, 0);
    let (o3_wat, _) = emit_source(COPY_F64, 3);
    let o0_loop = o0_wat
        .split_once("loop $ik_dispatch")
        .expect("O0 dispatcher loop")
        .1
        .split_once("br $ik_dispatch")
        .expect("O0 loop backedge")
        .0;
    let o3_loop = o3_wat
        .split_once("loop $ik_loop")
        .expect("O3 structured loop")
        .1
        .split_once("br $ik_loop")
        .expect("O3 loop backedge")
        .0;

    assert_eq!(o0_loop.matches("i32.mul").count(), 2, "{o0_loop}");
    assert_eq!(
        o3_loop.matches("i32.mul").count(),
        0,
        "the O3 f64 loop should use cursors instead of recalculating two 8-byte scaled addresses:\n{o3_loop}"
    );
}

#[test]
fn wasm_f64_cursor_should_match_o0_for_nonzero_start_and_wasm32_wrapping() {
    if !node_available() {
        return;
    }

    const RUNNER: &str = r#"
const fs = require("node:fs");
WebAssembly.instantiate(fs.readFileSync(process.argv[1]))
  .then(({ instance }) => {
    const { memory, copy_f64 } = instance.exports;
    const values = new Float64Array(memory.buffer);

    values[34] = 1.25; values[35] = -2.5; values[36] = 4.75;
    copy_f64(128, 256, 2, 5);
    if (values[18] !== 1.25 || values[19] !== -2.5 || values[20] !== 4.75) process.exit(1);

    // index * sizeof(f64) wraps to zero in the Wasm32 address calculation.
    values[0] = 8.5;
    copy_f64(512, 0, 0x20000000, 0x20000001);
    if (values[64] !== 8.5) process.exit(2);

    // Both base-plus-index expressions wrap while preserving the copied value.
    values[0] = -13.75;
    copy_f64(0xfffffff8, 0xfffffff0, 2, 3);
    if (values[1] !== -13.75) process.exit(3);
  })
  .catch((error) => { console.error(error); process.exit(4); });
"#;

    for opt_level in [0, 3] {
        let (_, wasm) = emit_source(COPY_F64, opt_level);
        let id = NEXT_WASM_MEMORY_ID.fetch_add(1, Ordering::Relaxed);
        let wasm_path = std::env::temp_dir().join(format!(
            "calckernel_wasm_memory_f64_cursor_{}_o{opt_level}.wasm",
            id
        ));
        fs::write(&wasm_path, wasm).expect("write Wasm module");
        let output = Command::new("node")
            .arg("-e")
            .arg(RUNNER)
            .arg(&wasm_path)
            .output()
            .expect("run Node Wasm harness");
        let _ = fs::remove_file(&wasm_path);
        assert!(
            output.status.success(),
            "O{opt_level} Node runtime failed with {:?}:\n{}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn wasm_o3_should_keep_struct_field_access_out_of_the_primitive_cursor_subset() {
    let (o3_wat, _) = emit_source(COPY_SLOT_FIELD, 3);
    let copy = o3_wat
        .split_once("(func $copy_slot_value")
        .expect("struct field copy function")
        .1
        .split_once("(@custom")
        .expect("module metadata follows functions")
        .0;
    let loop_body = copy
        .split_once("loop $ik_loop")
        .expect("O3 structured loop")
        .1
        .split_once("br $ik_loop")
        .expect("O3 loop backedge")
        .0;
    let normalized_loop = loop_body
        .lines()
        .map(str::trim)
        .collect::<Vec<_>>()
        .join("\n");

    assert_eq!(
        normalized_loop.matches("i32.mul").count(),
        2,
        "the presence of a nested field access keeps the entire loop on indexed addressing:\n{loop_body}"
    );
    assert!(
        normalized_loop.contains("i32.const 16\ni32.mul\ni32.add"),
        "the unsupported struct stride must retain indexed source addressing:\n{loop_body}"
    );
    assert!(
        normalized_loop.contains("i32.const 4\ni32.add\ni32.load offset=0 align=4"),
        "the unsupported field offset must retain its explicit address addition:\n{loop_body}"
    );
}

#[test]
fn wasm_o3_should_keep_indexed_addresses_for_a_multi_exit_loop() {
    let (o3_wat, _) = emit_copy(3);
    let copy_skip = o3_wat
        .split_once("(func $copy_skip")
        .expect("multi-exit copy function")
        .1
        .split_once("(@custom")
        .expect("module metadata follows functions")
        .0;

    assert!(copy_skip.contains("loop $ik_loop_b"), "{copy_skip}");
    assert!(
        copy_skip.contains("if\n"),
        "expected conditional continue CFG:\n{copy_skip}"
    );
    assert_eq!(
        copy_skip.matches("i32.mul").count(),
        2,
        "unsupported multi-exit CFG must keep its original indexed load/store addresses:\n{copy_skip}"
    );
}

#[test]
fn wasm_cursor_plan_should_reject_a_loop_backedge_that_replaces_the_source_descriptor() {
    let (wat, _) = emit_mutated_copy_switch(3);
    let copy_switch = wat
        .split_once("(func $copy_switch")
        .expect("copy_switch function")
        .1
        .split_once("(@custom")
        .expect("module metadata follows functions")
        .0;
    let loop_body = copy_switch
        .split_once("loop $ik_loop")
        .expect("structured copy loop")
        .1
        .split_once("br $ik_loop")
        .expect("copy loop backedge")
        .0;

    assert_eq!(
        loop_body.matches("i32.mul").count(),
        2,
        "a backedge that changes src from the original pointer to alternate must retain indexed load/store addressing:\n{loop_body}"
    );
}

#[test]
fn kir_validator_should_reject_using_a_loop_header_as_the_function_entry() {
    let optimized = optimize_source(COPY_U32, 3);
    let mut module = verified_artifact(&optimized).clone();
    let function = module
        .functions
        .iter_mut()
        .find(|function| function.name == "copy_u32")
        .expect("copy_u32 function");
    let header_index = function
        .blocks
        .iter()
        .position(|block| {
            matches!(block.terminator, KirTerminator::Branch { .. })
                && block.params.iter().any(|param| param.slot == "i")
        })
        .expect("loop header block");
    function.blocks.swap(0, header_index);

    let errors = calckernel::validate_kir_module(&module).errors;
    assert!(
        !errors.is_empty(),
        "a loop header with incoming loop-carried arguments cannot serve as a function entry"
    );
}

#[test]
fn wasm_cursor_plan_should_not_treat_a_loop_body_as_an_implicit_entry_edge() {
    let optimized = optimize_source(COPY_U32, 3);
    let mut module = verified_artifact(&optimized).clone();
    let function = module
        .functions
        .iter_mut()
        .find(|function| function.name == "copy_u32")
        .expect("copy_u32 function");
    let loop_header = function
        .blocks
        .iter()
        .find(|block| {
            matches!(block.terminator, KirTerminator::Branch { .. })
                && block.params.iter().any(|param| param.slot == "i")
        })
        .expect("loop header")
        .id;
    let body_index = function
        .blocks
        .iter()
        .position(|block| {
            block.instructions.iter().any(|instruction| {
                matches!(
                    instruction.kind,
                    calckernel::KirInstructionKind::Store { .. }
                )
            }) && matches!(
                &block.terminator,
                KirTerminator::Jump { edge } if edge.target == loop_header
            )
        })
        .expect("memory-accessing loop body with a backedge");
    function.blocks.swap(0, body_index);

    let validation = calckernel::validate_kir_module(&module);
    if validation.errors.is_empty() {
        let emitted = emit_wat_kir_module(&module, EmitWasmOptions { opt_level: 3 });
        assert!(
            emitted.is_err()
                || emitted
                    .as_ref()
                    .is_ok_and(|wat| !wat.contains("$ik_mem_cursor")),
            "an implicit function-entry edge into the loop body bypasses cursor initialization and must be rejected or retain indexed memory:\n{emitted:?}"
        );
    }
}

#[test]
fn wasm_mutated_source_descriptor_backedge_should_match_o0_and_o3() {
    if !node_available() {
        return;
    }

    const RUNNER: &str = r#"
const fs = require("node:fs");
WebAssembly.instantiate(fs.readFileSync(process.argv[1]))
  .then(({ instance }) => {
    const { memory, copy_switch } = instance.exports;
    const words = new Uint32Array(memory.buffer);
    words[100] = 0x11110000;
    words[101] = 0x11110001;
    words[102] = 0x11110002;
    words[300] = 0x22220000;
    words[301] = 0x22220001;
    words[302] = 0x22220002;
    copy_switch(800, 400, 1200, 0, 3);
    if (words[200] !== 0x11110000) process.exit(1);
    if (words[201] !== 0x22220001 || words[202] !== 0x22220002) process.exit(2);
  })
  .catch((error) => { console.error(error); process.exit(3); });
"#;

    for opt_level in [0, 3] {
        let (_, wasm) = emit_mutated_copy_switch(opt_level);
        let id = NEXT_WASM_MEMORY_ID.fetch_add(1, Ordering::Relaxed);
        let wasm_path = std::env::temp_dir().join(format!(
            "calckernel_wasm_memory_mutated_base_{}_o{opt_level}.wasm",
            id
        ));
        fs::write(&wasm_path, wasm).expect("write Wasm module");
        let output = Command::new("node")
            .arg("-e")
            .arg(RUNNER)
            .arg(&wasm_path)
            .output()
            .expect("run Node Wasm harness");
        let _ = fs::remove_file(&wasm_path);
        assert!(
            output.status.success(),
            "O{opt_level} Node runtime failed with {:?}:\n{}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn wasm_o3_should_fold_only_an_offset_with_a_checked_alignment_no_wrap_proof() {
    for features in ["baseline", "simd128"] {
        let o3_wat = emit_wat_through_ckc(SLOT_OFFSETS, 3, features);
        let aligned = o3_wat
            .split_once("(func $load_aligned")
            .expect("aligned field loader")
            .1
            .split_once("(func $load_unaligned")
            .expect("unaligned field loader follows")
            .0;
        let unaligned = o3_wat
            .split_once("(func $load_unaligned")
            .expect("unaligned field loader")
            .1
            .split_once("(@custom")
            .expect("module metadata follows functions")
            .0;

        assert!(
            aligned.contains("i32.load offset=4 align=4"),
            "{features}: aligned(items, 16) and 16-byte index stride prove that adding field offset 4 cannot wrap:\n{aligned}"
        );
        assert!(
            !aligned.contains("i32.const 4\n    i32.add"),
            "{features}: the proven field offset should be carried by memarg:\n{aligned}"
        );

        assert!(
            unaligned.contains("i32.const 4\n    i32.add\n    i32.load offset=0 align=4"),
            "{features}: without an alignment proof, retain the Wasm32-wrapping address addition:\n{unaligned}"
        );
    }
}

#[test]
fn wasm_o1_o2_should_keep_field_offsets_explicit() {
    for opt_level in [1, 2] {
        let wat = emit_wat_through_ckc(SLOT_OFFSETS, opt_level, "baseline");
        let aligned = wat
            .split_once("(func $load_aligned")
            .expect("aligned field loader")
            .1
            .split_once("(func $load_unaligned")
            .expect("unaligned field loader follows")
            .0;

        assert!(
            aligned.contains("i32.const 4\n    i32.add\n    i32.load offset=0 align=4"),
            "O{opt_level} should retain the explicit, wrapping field address:\n{aligned}"
        );
        assert!(
            !aligned.contains("i32.load offset=4 align=4"),
            "O{opt_level} should not apply the P8b offset fold:\n{aligned}"
        );
    }
}

#[test]
fn wasm_nested_field_without_alignment_should_keep_field_address_explicit() {
    let wat = emit_wat_through_ckc(NESTED_FIELD, 3, "baseline");
    let nested = wat
        .split_once("(func $load_nested")
        .expect("nested field loader")
        .1
        .split_once("(@custom")
        .expect("module metadata follows functions")
        .0;
    let normalized = nested.lines().map(str::trim).collect::<Vec<_>>().join("\n");

    assert!(
        normalized.contains("i32.const 24\ni32.mul\ni32.add"),
        "the nested record stride must remain in explicit address arithmetic:\n{nested}"
    );
    assert!(
        normalized
            .contains("i32.const 8\ni32.add\ni32.const 4\ni32.add\ni32.load offset=0 align=4"),
        "without a checked alignment fact, the nested field offset must not move to memarg:\n{nested}"
    );
    assert!(
        !normalized.contains("i32.load offset=8 align=4"),
        "nested field layout alone must not authorize a memarg offset:\n{nested}"
    );
}

#[test]
fn wasm_slice_paired_abi_should_keep_unsupported_field_offset_conservative() {
    for (source, function_name) in [
        (SLOT_WITH_UNUSED_SLICE, "load_with_slice_spare"),
        (SLOT_WITH_POINTER_SLICE_SPARE, "load_with_pointer_spare"),
    ] {
        let wat = emit_wat_through_ckc(source, 3, "baseline");
        let marker = format!("(func ${function_name}");
        let function = wat
            .split_once(&marker)
            .expect("paired-slice field loader")
            .1
            .split_once("(@custom")
            .expect("module metadata follows functions")
            .0;

        assert!(
            function.contains("i32.const 4\n    i32.add\n    i32.load offset=0 align=4"),
            "unsupported paired-slice ABI should fall back to explicit field addressing:\n{function}"
        );
        assert!(
            !function.contains("i32.load offset=4 align=4"),
            "unsupported paired-slice ABI must not apply the checked memarg plan:\n{function}"
        );
    }
}

#[test]
fn wasm_bare_kir_should_keep_field_offsets_explicit_without_checked_facts() {
    let optimized = optimize_source(SLOT_OFFSETS, 3);
    let bare_wat = emit_wat_kir_module(
        verified_artifact(&optimized),
        EmitWasmOptions { opt_level: 3 },
    )
    .expect("bare verified KIR should emit conservative WAT");
    let aligned = bare_wat
        .split_once("(func $load_aligned")
        .expect("aligned field loader")
        .1
        .split_once("(func $load_unaligned")
        .expect("unaligned field loader follows")
        .0;

    assert!(
        aligned.contains("i32.const 4\n    i32.add\n    i32.load offset=0 align=4"),
        "bare KIR carries no contract proof, so the Wasm32 wrapping addition stays explicit:\n{aligned}"
    );
    assert!(
        !aligned.contains("i32.load offset=4 align=4"),
        "bare KIR must not infer a no-wrap proof from the source type or field layout:\n{aligned}"
    );
}

#[test]
fn wasm_result_emitter_should_reject_a_mutated_artifact() {
    let mut result = optimize_source(SLOT_OFFSETS, 3);
    result
        .artifact
        .as_mut()
        .expect("verified artifact")
        .functions[0]
        .name
        .push_str("_tampered");

    assert!(
        calckernel::emit_wat_kir_result(&result, EmitWasmOptions { opt_level: 3 }).is_err(),
        "a changed artifact must not reuse cached address proofs"
    );
    assert!(
        calckernel::emit_wasm_kir_result(&result, EmitWasmOptions { opt_level: 3 }).is_err(),
        "binary emission must reject the same changed artifact"
    );
}

#[test]
fn wasm_result_emitter_should_reject_tampered_alignment_fact_provenance() {
    let mut result = optimize_source(SLOT_OFFSETS, 3);
    result
        .contract_facts
        .as_mut()
        .expect("checked contract facts")
        .facts_mut()
        .get_mut(calckernel::FactId::from_index(0))
        .expect("alignment fact")
        .generation += 1;

    assert!(
        calckernel::emit_wat_kir_result(&result, EmitWasmOptions { opt_level: 3 }).is_err(),
        "a changed alignment fact must not reuse cached address proofs"
    );
    assert!(
        calckernel::emit_wasm_kir_result(&result, EmitWasmOptions { opt_level: 3 }).is_err(),
        "binary emission must reject changed proof provenance"
    );
}

#[test]
fn wasm_memarg_offset_fold_should_match_o0_at_normal_and_wraparound_addresses() {
    if !node_available() {
        return;
    }

    const RUNNER: &str = r#"
const fs = require("node:fs");
WebAssembly.instantiate(fs.readFileSync(process.argv[1]))
  .then(({ instance }) => {
    const { memory, load_aligned, load_unaligned } = instance.exports;
    const words = new Uint32Array(memory.buffer);

    words[65] = 0x24681357;
    if (load_aligned(256, 0) !== 0x24681357) process.exit(1);

    words[69] = 0x1234abcd;
    if (load_aligned(256, 1) !== 0x1234abcd) process.exit(2);

    // The last slot ends exactly at the current memory boundary.
    words[16381] = 0x31415926;
    if (load_aligned(65520, 0) !== 0x31415926) process.exit(3);

    // The first slot past the memory end must trap.
    let onePastTrapped = false;
    try { load_aligned(65520, 1); }
    catch (error) { onePastTrapped = error instanceof WebAssembly.RuntimeError; }
    if (!onePastTrapped) process.exit(4);

    // The 16-byte stride wraps modulo 2^32 while preserving 16-byte alignment.
    words[65] = 0x5678ef01;
    if (load_aligned(256, 0x10000000) !== 0x5678ef01) process.exit(5);

    // A negative i32 bit pattern is still interpreted modulo 2^32 for indexing.
    words[61] = 0x76543210;
    if (load_aligned(256, 0xffffffff) !== 0x76543210) process.exit(8);

    // Without aligned(items, 16), adding the field offset wraps to address 0.
    words[0] = 0x9abcdef0;
    if ((load_unaligned(0xfffffffc, 0) >>> 0) !== 0x9abcdef0) process.exit(6);

    // A proven-aligned residual can still be out of memory; memarg must trap.
    let trapped = false;
    try { load_aligned(0xfffffff0, 0); }
    catch (error) { trapped = error instanceof WebAssembly.RuntimeError; }
    if (!trapped) process.exit(7);
  })
  .catch((error) => { console.error(error); process.exit(5); });
"#;

    for features in ["baseline", "simd128"] {
        for opt_level in [0, 3] {
            let wasm = emit_wasm_through_ckc(SLOT_OFFSETS, opt_level, features);
            let id = NEXT_WASM_MEMORY_ID.fetch_add(1, Ordering::Relaxed);
            let wasm_path = std::env::temp_dir().join(format!(
                "calckernel_wasm_memarg_offset_{}_{}_o{opt_level}.wasm",
                features, id
            ));
            fs::write(&wasm_path, wasm).expect("write Wasm module");
            let output = Command::new("node")
                .arg("-e")
                .arg(RUNNER)
                .arg(&wasm_path)
                .output()
                .expect("run Node Wasm harness");
            let _ = fs::remove_file(&wasm_path);
            assert!(
                output.status.success(),
                "{features} O{opt_level} Node runtime failed with {:?}:\n{}",
                output.status.code(),
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
}
