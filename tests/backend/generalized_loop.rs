use calckernel::{
    EmitWasmOptions, KirBoundsMode, KirBuildConfig, KirConsumer, KirOptimizationLevel,
    KirOverflowMode, KirSanitizerMode, KirTargetProfile, KirVerifiedProgramState, KirWasmFeatures,
    SourceFile, build_kir_module_with_profile, check, emit_wasm_kir_module, import_contract_facts,
    lower_to_mir, run_kir_pass_pipeline,
};
use std::{fs, process::Command};

const SIMD_FEATURES: KirWasmFeatures = KirWasmFeatures::Simd128;

#[test]
fn unknown_alias_f64_guards_preserve_disjoint_overlap_tail_and_trap_prefix_results() {
    let source = r#"
export unsafe fn update(a: slice<f64>, out: slice<f64>, n: u32) -> void
contract {
  requires n <= a.len && n <= out.len;
  effects read(a), readwrite(out);
} {
  let i: u32 = 0;
  while i < n {
    let previous: f64 = out[i];
    out[i] = previous + a[i] + 7.0;
    i = i + 1;
  }
}
"#;
    let (o0, _) = emit(source, KirOptimizationLevel::O0, KirWasmFeatures::Baseline);
    let (baseline, _) = emit(source, KirOptimizationLevel::O3, KirWasmFeatures::Baseline);
    let (simd, vectorized) = emit(source, KirOptimizationLevel::O3, SIMD_FEATURES);
    assert_eq!(
        vectorized, 1,
        "two-slice unknown-alias f64 remains optimized"
    );
    if !crate::support::command::node_available() {
        eprintln!("skipping f64 range/alias runtime differential: Node is unavailable");
        return;
    }
    let directory = crate::support::temp::temp_dir("ck-f64-range-alias");
    fs::create_dir_all(&directory).expect("runtime directory");
    for (name, bytes) in [("o0", o0), ("baseline", baseline), ("simd", simd)] {
        fs::write(directory.join(format!("{name}.wasm")), bytes).expect("write module");
    }
    let script = r#"
const fs = require('node:fs'), assert = require('node:assert/strict');
const engines = process.argv.slice(1).map(path =>
  new WebAssembly.Instance(new WebAssembly.Module(fs.readFileSync(path))).exports);
const scenarios = [];
for (const n of [0, 1, 2, 3, 4, 7, 8, 9, 15, 16, 17, 32]) {
  for (const delta of [null, -8, 0, 8]) scenarios.push({ n, delta, near: null });
}
for (const n of [2, 3, 8]) for (const near of ['input', 'output']) {
  scenarios.push({ n, delta: null, near });
}
let checks = 0;
for (const scenario of scenarios) {
  let expected;
  for (const engine of engines) {
    const bytes = new Uint8Array(engine.memory.buffer);
    bytes.fill(0xa5);
    const view = new DataView(engine.memory.buffer);
    const a = scenario.near === 'input' ? bytes.length - 8 : 1024;
    const out = scenario.near === 'output' ? bytes.length - 8
      : scenario.delta === null ? 4096 : a + scenario.delta;
    for (let i = 0; i < 64; i++) for (const base of [a, out]) {
      if (base + i * 8 + 8 <= bytes.length) {
        view.setFloat64(base + i * 8, (i % 17 - 8) / 4, true);
      }
    }
    const initial = Buffer.from(bytes);
    let trapped = false;
    try { engine.update(a, 64, out, 64, scenario.n); }
    catch (error) { assert(error instanceof WebAssembly.RuntimeError); trapped = true; }
    const result = { initial, trapped, bytes: Buffer.from(bytes) };
    if (expected) {
      assert(result.initial.equals(expected.initial), 'initial memory differs');
      assert.equal(result.trapped, expected.trapped, JSON.stringify(scenario));
      assert(result.bytes.equals(expected.bytes), `write prefix differs: ${JSON.stringify(scenario)}`);
      checks++;
    } else {
      expected = result;
      if (scenario.near) {
        assert(trapped, 'near-end source must trap after its first iteration');
        assert(!result.bytes.equals(initial), 'scalar completed-write prefix must be observable');
      }
    }
  }
}
process.stdout.write(String(checks));
"#;
    let result = Command::new("node")
        .args(["-e", script])
        .args(["o0", "baseline", "simd"].map(|name| directory.join(format!("{name}.wasm"))))
        .output()
        .expect("run f64 alias/range differential");
    let _ = fs::remove_dir_all(&directory);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8(result.stdout).unwrap(), "108");
}

fn optimize(
    source: &str,
    level: KirOptimizationLevel,
    features: KirWasmFeatures,
) -> (KirVerifiedProgramState, u32) {
    let checked = check(&SourceFile::new("generalized-loop.ck", source));
    assert_eq!(checked.diagnostics, [], "invalid test source:\n{source}");
    let mir = lower_to_mir(&checked.checked_program).expect("MIR lowering");
    let module = build_kir_module_with_profile(
        &mir,
        KirBuildConfig {
            consumer: KirConsumer::WebAssembly,
            overflow_mode: KirOverflowMode::Unchecked,
            bounds_mode: KirBoundsMode::Unchecked,
            sanitizer_mode: KirSanitizerMode::Disabled,
        },
        KirTargetProfile::webassembly_with_features(features),
    )
    .expect("WASM KIR");
    let contracts =
        import_contract_facts(&module, &checked.checked_program, 0).expect("contract facts");
    let optimized = run_kir_pass_pipeline(module, level, Some(&contracts));
    assert!(optimized.errors.is_empty(), "{:?}", optimized.errors);
    let vectorized = optimized.stats.vectorized_loops;
    let state = KirVerifiedProgramState::from_parts(
        optimized.artifact.expect("optimized KIR artifact"),
        optimized.contract_facts,
        optimized.proofs,
        optimized.eliminated_guards,
        0,
    )
    .expect("verified optimized state");
    (state, vectorized)
}

fn emit(source: &str, level: KirOptimizationLevel, features: KirWasmFeatures) -> (Vec<u8>, u32) {
    let (state, vectorized) = optimize(source, level, features);
    let opt_level = match level {
        KirOptimizationLevel::O0 => 0,
        KirOptimizationLevel::O1 => 1,
        KirOptimizationLevel::O2 => 2,
        KirOptimizationLevel::O3 => 3,
    };
    let bytes =
        emit_wasm_kir_module(state.module(), EmitWasmOptions { opt_level }).expect("WASM emission");
    (bytes, vectorized)
}

fn source_variant(seed: u32) -> String {
    // Deterministic identifier and instruction-layout changes keep these cases
    // deterministic while varying source shape independently of kernel names.
    let (left, right, weight, sum, product, previous, output) = match seed {
        0 => ("lhs", "rhs", "scale", "pair", "term", "prior", "dst"),
        1 => (
            "first", "second", "gain", "combined", "scaled", "old", "target",
        ),
        2 => ("x", "y", "factor", "total", "weighted", "before", "result"),
        _ => (
            "sample_a",
            "sample_b",
            "coefficient",
            "joined",
            "mapped",
            "carry",
            "answer",
        ),
    };
    let body = match seed {
        0 => format!(
            "let {left}: f64 = a[ao + i];\n             let {right}: f64 = b[bo + i];\n             let {weight}: f64 = a[ao];\n             let {sum}: f64 = {left} + {right};\n             let {product}: f64 = {sum} * {weight};\n             let oi: u32 = oo + i;\n             let {previous}: f64 = out[oi];\n             out[oi] = {previous} + {product};"
        ),
        1 => format!(
            "let {weight}: f64 = a[ao];\n             let {right}: f64 = b[bo + i];\n             let copied: f64 = a[ao + i];\n             let {left}: f64 = copied;\n             let {sum}: f64 = {left} + {right};\n             let oi: u32 = oo + i;\n             let {previous}: f64 = out[oi];\n             let {product}: f64 = {sum} * {weight};\n             out[oi] = {previous} + {product};"
        ),
        2 => format!(
            "let ai: u32 = ao + i;\n             let oi: u32 = oo + i;\n             let {left}: f64 = a[ai];\n             let {right}: f64 = b[bo + i];\n             let {weight}: f64 = a[ao];\n             let {sum}: f64 = {left} + {right};\n             let {previous}: f64 = out[oi];\n             let {product}: f64 = {sum} * {weight};\n             out[oi] = {previous} + {product};"
        ),
        _ => format!(
            "let oi: u32 = oo + i;\n             let {previous}: f64 = out[oi];\n             let {weight}: f64 = a[ao];\n             let {sum}: f64 = a[ao + i] + b[bo + i];\n             let {product}: f64 = {sum} * {weight};\n             let {output}: f64 = {previous} + {product};\n             out[oi] = {output};"
        ),
    };
    format!(
        r#"
export unsafe fn run(
  a: slice<f64>, b: slice<f64>, out: slice<f64>,
  n: u32, ao: u32, bo: u32, oo: u32
) -> void contract {{
  requires n <= a.len && ao <= a.len - n;
  requires n <= b.len && bo <= b.len - n;
  requires n <= out.len && oo <= out.len - n;
  requires noalias(a, out) && noalias(b, out);
  effects read(a), read(b), readwrite(out);
}} {{
  let i: u32 = 0;
  while i < n {{
    {body}
    i = i + 1;
  }}
}}
"#
    )
}

#[test]
fn generic_loop_variants_keep_o0_o3_and_simd128_results_equivalent() {
    let variants = [0, 1, 2, 3].map(source_variant);
    let mut modules = Vec::new();
    let mut vectorized_count = 0;
    for source in &variants {
        let (o0, _) = emit(source, KirOptimizationLevel::O0, KirWasmFeatures::Baseline);
        let (baseline_o3, _) = emit(source, KirOptimizationLevel::O3, KirWasmFeatures::Baseline);
        let (simd_o3, vectorized) = emit(source, KirOptimizationLevel::O3, SIMD_FEATURES);
        vectorized_count += vectorized;
        modules.extend([o0, baseline_o3, simd_o3]);
    }
    assert_eq!(
        vectorized_count,
        variants.len() as u32,
        "each accepted generic source form should vectorize"
    );

    if !crate::support::command::node_available() {
        eprintln!("skipping generalized WASM differential corpus: Node is unavailable");
        return;
    }
    let dir = crate::support::temp::temp_dir("ck-generalized-loop-corpus");
    fs::create_dir_all(&dir).expect("temporary corpus directory");
    for (index, bytes) in modules.iter().enumerate() {
        fs::write(dir.join(format!("module-{index}.wasm")), bytes).expect("write WASM module");
    }
    fs::write(
        dir.join("check.cjs"),
        r#"
const fs = require('node:fs');
const assert = require('node:assert/strict');
const paths = process.argv.slice(2);
const modules = paths.map(path => new WebAssembly.Module(fs.readFileSync(path)));
const tripCounts = [0, 1, 2, 3, 4, 5, 7, 8, 9, 15, 16, 17];
const edgeBits = [
  0x0000000000000000n, 0x8000000000000000n,
  0x3ff0000000000000n, 0xbff0000000000000n,
  0x7ff0000000000000n, 0xfff0000000000000n,
  0x7ff8000000000042n, 0x7ff0000000000001n,
  0x400921fb54442d18n, 0xc005bf0a8b145769n
];
let randomState = 0x5eed1234;
function nextRandom() {
  randomState ^= randomState << 13;
  randomState ^= randomState >>> 17;
  randomState ^= randomState << 5;
  return randomState >>> 0;
}
const scenarios = [];
for (const n of tripCounts) for (const [ao, bo, oo] of [[0, 0, 0], [1, 3, 5], [7, 1, 2]]) {
  scenarios.push({ n, ao, bo, oo, inputSeed: null });
}
for (let sample = 0; sample < 64; sample++) {
  const n = nextRandom() % 49;
  const room = 65 - n;
  scenarios.push({ n, ao: nextRandom() % room, bo: nextRandom() % room,
    oo: nextRandom() % room, inputSeed: nextRandom() });
}
function equalMemory(expected, actual, out, outOffset, count, label) {
  const a = new DataView(expected.buffer, expected.byteOffset, expected.byteLength);
  const b = new DataView(actual.buffer, actual.byteOffset, actual.byteLength);
  const output = new Set();
  for (let i = 0; i < count; i++) output.add((out + (outOffset + i) * 8) >>> 0);
  for (let offset = 0; offset < expected.length; offset += 8) {
    const x = a.getBigUint64(offset, true), y = b.getBigUint64(offset, true);
    if (x === y) continue;
    const arithmeticNaN = output.has(offset)
      && Number.isNaN(a.getFloat64(offset, true))
      && Number.isNaN(b.getFloat64(offset, true))
      && (x & 0x0008000000000000n) !== 0n
      && (y & 0x0008000000000000n) !== 0n;
    assert(arithmeticNaN, `${label}: changed memory at byte ${offset}: ${x.toString(16)} != ${y.toString(16)}`);
  }
}
let cases = 0;
for (let variant = 0; variant < modules.length / 3; variant++) {
  const engines = modules.slice(variant * 3, variant * 3 + 3).map(module =>
    new WebAssembly.Instance(module).exports);
  for (const { n, ao, bo, oo, inputSeed } of scenarios) {
    let expected;
    for (const [engineIndex, wasm] of engines.entries()) {
      const memory = new Uint8Array(wasm.memory.buffer);
      memory.fill(0xa5);
      const view = new DataView(wasm.memory.buffer);
      const a = 1024, b = 4096, out = 10240, len = 64;
      if (inputSeed !== null) randomState = inputSeed;
      for (let i = 0; i < len; i++) {
        if (inputSeed === null) {
          view.setBigUint64(a + i * 8, edgeBits[(i + 1) % edgeBits.length], true);
          view.setBigUint64(b + i * 8, edgeBits[(i + 4) % edgeBits.length], true);
          view.setBigUint64(out + i * 8, edgeBits[(i + 2) % edgeBits.length], true);
        } else {
          for (const base of [a, b, out]) {
            view.setFloat64(base + i * 8, ((nextRandom() % 4097) - 2048) / 32, true);
          }
        }
      }
      const before = Buffer.from(memory);
      let trapped = false;
      try { wasm.run(a, len, b, len, out, len, n, ao, bo, oo); }
      catch (error) { assert(error instanceof WebAssembly.RuntimeError); trapped = true; }
      const result = { trapped, bytes: Buffer.from(memory), before };
      if (expected) {
        assert.equal(result.trapped, expected.trapped, `trap disagreement variant=${variant} n=${n}`);
        equalMemory(expected.bytes, result.bytes, out, oo, n, `variant=${variant} engine=${engineIndex} n=${n}`);
      } else expected = result;
      cases++;
    }
  }
}
process.stdout.write(String(cases));
"#,
    )
    .expect("write Node differential runner");
    let result = Command::new("node")
        .arg(dir.join("check.cjs"))
        .args((0..modules.len()).map(|index| dir.join(format!("module-{index}.wasm"))))
        .output()
        .expect("run Node differential corpus");
    let _ = fs::remove_dir_all(&dir);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8(result.stdout).unwrap(), "1200");
}

#[test]
fn unknown_overlapping_input_output_alias_keeps_sequential_memory_results() {
    let source = r#"
export unsafe fn run(a: slice<u32>, out: slice<u32>, n: u32) -> void contract {
  requires n <= a.len && n <= out.len;
  effects read(a), readwrite(out);
} {
  let i: u32 = 0;
  while i < n {
    let input: u32 = a[i];
    let previous: u32 = out[i];
    out[i] = previous + input;
    i = i + 1;
  }
}
"#;
    let (o0, _) = emit(source, KirOptimizationLevel::O0, KirWasmFeatures::Baseline);
    let (baseline_o3, _) = emit(source, KirOptimizationLevel::O3, KirWasmFeatures::Baseline);
    let (simd_o3, _) = emit(source, KirOptimizationLevel::O3, SIMD_FEATURES);
    if !crate::support::command::node_available() {
        eprintln!("skipping overlapping-alias WASM differential check: Node is unavailable");
        return;
    }
    let dir = crate::support::temp::temp_dir("ck-generalized-loop-alias");
    fs::create_dir_all(&dir).expect("temporary alias directory");
    for (name, bytes) in [
        ("o0", o0),
        ("baseline-o3", baseline_o3),
        ("simd-o3", simd_o3),
    ] {
        fs::write(dir.join(format!("{name}.wasm")), bytes).expect("write alias module");
    }
    fs::write(
        dir.join("check.cjs"),
        r#"
const fs = require('node:fs'), assert = require('node:assert/strict');
const engines = process.argv.slice(2).map(path => new WebAssembly.Instance(
  new WebAssembly.Module(fs.readFileSync(path))).exports);
for (const n of [0, 1, 2, 3, 4, 7, 8, 9, 17]) {
  for (const delta of [-1, 0, 1]) {
    let expected;
    for (const wasm of engines) {
      const memory = new Uint8Array(wasm.memory.buffer);
      memory.fill(0x5a);
      const words = new Uint32Array(wasm.memory.buffer);
      for (let i = 0; i < words.length; i++) words[i] = (Math.imul(i, 0x9e3779b1) + 0x1234567) >>> 0;
      const a = 4096, out = a + delta * 4, len = 32;
      const before = Buffer.from(memory);
      let trapped = false;
      try { wasm.run(a, len, out, len, n); }
      catch (error) { assert(error instanceof WebAssembly.RuntimeError); trapped = true; }
      const result = { trapped, bytes: Buffer.from(memory) };
      if (expected) {
        assert.equal(result.trapped, expected.trapped, `trap status n=${n} delta=${delta}`);
        assert(result.bytes.equals(expected.bytes), `memory mismatch n=${n} delta=${delta}`);
      } else expected = result;
    }
  }
}
console.log('27');
"#,
    )
    .expect("write alias Node runner");
    let result = Command::new("node")
        .arg(dir.join("check.cjs"))
        .args(["o0", "baseline-o3", "simd-o3"].map(|name| dir.join(format!("{name}.wasm"))))
        .output()
        .expect("run alias Node corpus");
    let _ = fs::remove_dir_all(&dir);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8(result.stdout).unwrap().trim(), "27");
}

#[test]
fn negative_loop_memory_shapes_fail_closed_without_panicking() {
    let sources = [
        r#"export unsafe fn run(a: slice<u32>, out: slice<u32>, n: u32) -> void contract {
  requires n <= a.len && n <= out.len; effects read(a), write(out);
} { let i: u32 = 0; while i < n { out[i] = a[i * i]; i = i + 1; } }"#,
        r#"export unsafe fn run(a: slice<u32>, out: slice<u32>, n: u32, stride: u32) -> void contract {
  requires n <= out.len; effects read(a), write(out);
} { let i: u32 = 0; while i < n { out[i] = a[i * stride]; i = i + 1; } }"#,
        r#"export unsafe fn run(a: slice<u32>, out: slice<u32>, n: u32) -> void contract {
  requires n <= a.len && n <= out.len; effects read(a), readwrite(out);
} { let i: u32 = 1; while i < n { out[i] = out[i - 1] + a[i]; i = i + 1; } }"#,
        r#"export unsafe fn run(a: slice<u32>, out: slice<u32>, n: u32) -> void contract {
  requires n <= a.len && n < out.len; effects read(a), readwrite(out);
} { let i: u32 = 0; while i < n { let next: u32 = out[i + 1]; out[i] = next + a[i]; i = i + 1; } }"#,
        r#"export unsafe fn run(a: slice<u32>, b: slice<u32>, out: slice<u32>, n: u32) -> void contract {
  requires n <= a.len && n <= b.len && n < out.len; effects read(a), read(b), write(out);
} { let i: u32 = 0; while i < n { out[i] = a[i]; out[i + 1] = b[i]; i = i + 1; } }"#,
        r#"export unsafe fn run(a: slice<u32>, out: slice<u32>, n: u32) -> void contract {
  requires n <= a.len && n <= out.len; effects read(a), write(out);
} { let i: u32 = 0; while i < n { out[a[i]] = i; i = i + 1; } }"#,
    ];
    for source in sources {
        let (_, vectorized) = optimize(source, KirOptimizationLevel::O3, SIMD_FEATURES);
        assert_eq!(
            vectorized, 0,
            "unsupported dependence/access must remain scalar:\n{source}"
        );
    }
}

#[test]
fn strict_float_reduction_checked_arithmetic_and_effectful_calls_stay_scalar() {
    let strict_reduction = r#"
export unsafe fn run(a: slice<f64>, n: u32) -> f64 contract {
  requires n <= a.len; effects read(a);
} {
  let i: u32 = 0;
  let total: f64 = 0.0;
  while i < n { total = total + a[i]; i = i + 1; }
  return total;
}
"#;
    assert_eq!(
        optimize(strict_reduction, KirOptimizationLevel::O3, SIMD_FEATURES).1,
        0,
        "strict floating reduction must preserve scalar association"
    );

    let checked = r#"
export unsafe fn run(a: slice<i32>, out: slice<i32>, n: u32) -> void contract {
  requires n <= a.len && n <= out.len; requires noalias(a, out);
  effects read(a), write(out);
} { let i: u32 = 0; while i < n { out[i] = a[i] + 1; i = i + 1; } }
"#;
    let checked_program = check(&SourceFile::new("checked-vector-loop.ck", checked));
    assert_eq!(checked_program.diagnostics, [], "invalid checked fixture");
    let mir = lower_to_mir(&checked_program.checked_program).expect("checked MIR lowering");
    let error = build_kir_module_with_profile(
        &mir,
        KirBuildConfig {
            consumer: KirConsumer::WebAssembly,
            overflow_mode: KirOverflowMode::Checked,
            bounds_mode: KirBoundsMode::Unchecked,
            sanitizer_mode: KirSanitizerMode::Disabled,
        },
        KirTargetProfile::webassembly_with_features(SIMD_FEATURES),
    )
    .expect_err("checked arithmetic must fail closed for this unsupported WASM profile");
    assert!(error.message.contains("checked overflow mode"), "{error}");

    let effectful = r#"
unsafe fn observe(trace: slice<i32>, index: u32, value: i32) -> i32 contract {
  requires index < trace.len; effects write(trace);
} { trace[index] = value; return value; }
export unsafe fn run(a: slice<i32>, out: slice<i32>, trace: slice<i32>, n: u32) -> void contract {
  requires n <= a.len && n <= out.len && n <= trace.len;
  requires noalias(a, out) && noalias(a, trace) && noalias(out, trace);
  effects read(a), write(out), write(trace);
} {
  let i: u32 = 0;
  while i < n {
    unsafe { out[i] = observe(trace, i, a[i]); }
    i = i + 1;
  }
}
"#;
    assert_eq!(
        optimize(effectful, KirOptimizationLevel::O3, SIMD_FEATURES).1,
        0,
        "ordered effectful calls must not enter SIMD"
    );
}
