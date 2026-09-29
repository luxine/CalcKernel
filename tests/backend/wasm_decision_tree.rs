use calckernel::{
    EmitWasmOptions, KirBuildConfig, KirConsumer, KirOptimizationLevel, KirOverflowMode,
    KirSanitizerMode, KirTargetProfile, KirVerifiedProgramState, KirWasmFeatures, SourceFile,
    build_kir_module_with_profile, check, check_decision_tree_vector_trial_independently,
    discover_wasm_decision_tree_candidates, emit_wasm_kir_module, emit_wat_kir_module,
    import_contract_facts, lower_to_mir, prepare_decision_tree_vector_trial, run_kir_pass_pipeline,
};
use std::{fs, process::Command};

const PIECEWISE_SOURCE: &str = r#"
export unsafe fn piecewise(input: slice<f64>, out: slice<f64>) -> void contract {
  requires input.len == out.len;
  requires noalias(input, out);
  effects read(input), write(out);
} {
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

fn optimized(source: &str, level: KirOptimizationLevel) -> calckernel::KirPassManagerResult {
    let checked = check(&SourceFile::new("wasm-decision-tree.ck", source));
    assert_eq!(checked.diagnostics, [], "invalid test source:\n{source}");
    let mir = lower_to_mir(&checked.checked_program).expect("MIR lowering");
    let module = build_kir_module_with_profile(
        &mir,
        KirBuildConfig {
            consumer: KirConsumer::WebAssembly,
            overflow_mode: KirOverflowMode::Unchecked,
            bounds_mode: calckernel::KirBoundsMode::Unchecked,
            sanitizer_mode: KirSanitizerMode::Disabled,
        },
        KirTargetProfile::webassembly_with_features(KirWasmFeatures::Simd128),
    )
    .expect("SIMD128 KIR");
    let contracts =
        import_contract_facts(&module, &checked.checked_program, 0).expect("contract facts");
    let result = run_kir_pass_pipeline(module, level, Some(&contracts));
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    result
}

fn emit(source: &str, level: KirOptimizationLevel) -> (calckernel::KirModule, String, Vec<u8>) {
    let optimized = optimized(source, level);
    let module = optimized.artifact.expect("verified KIR");
    let options = EmitWasmOptions {
        opt_level: if level == KirOptimizationLevel::O3 {
            3
        } else {
            0
        },
    };
    let wat = emit_wat_kir_module(&module, options).expect("WAT");
    let wasm = emit_wasm_kir_module(&module, options).expect("WASM");
    (module, wat, wasm)
}

fn emit_direct_uf4(source: &str) -> (calckernel::KirModule, String, Vec<u8>, u32) {
    let checked = check(&SourceFile::new("wasm-decision-tree-uf4.ck", source));
    assert_eq!(checked.diagnostics, [], "invalid test source:\n{source}");
    let mir = lower_to_mir(&checked.checked_program).expect("MIR lowering");
    let profile = KirTargetProfile::webassembly_with_features(KirWasmFeatures::Simd128);
    let module = build_kir_module_with_profile(
        &mir,
        KirBuildConfig {
            consumer: KirConsumer::WebAssembly,
            overflow_mode: KirOverflowMode::Unchecked,
            bounds_mode: calckernel::KirBoundsMode::Unchecked,
            sanitizer_mode: KirSanitizerMode::Disabled,
        },
        profile,
    )
    .expect("SIMD128 KIR");
    let mut optimized = run_kir_pass_pipeline(module, KirOptimizationLevel::O3, None);
    assert!(optimized.errors.is_empty(), "{:?}", optimized.errors);
    let module = optimized.artifact.take().expect("optimized scalar KIR");
    let contracts =
        import_contract_facts(&module, &checked.checked_program, 0).expect("contract facts");
    let state = KirVerifiedProgramState::from_parts(
        module,
        Some(contracts),
        optimized.proofs,
        optimized.eliminated_guards,
        0,
    )
    .expect("verified source state");
    let discovery = discover_wasm_decision_tree_candidates(&state);
    let candidate = discovery
        .candidates
        .iter()
        .find(|candidate| candidate.uf == 4)
        .unwrap_or_else(|| panic!("UF4 candidate: {discovery:?}"));
    let minimum_trip = candidate.minimum_trip;
    let prepared = prepare_decision_tree_vector_trial(&state, candidate).expect("UF4 trial");
    check_decision_tree_vector_trial_independently(
        &state,
        &prepared.trial,
        &prepared.plan,
        &prepared.charge,
    )
    .expect("independent UF4 checker");
    let module = prepared.trial.module().clone();
    let options = EmitWasmOptions { opt_level: 3 };
    let wat = emit_wat_kir_module(&module, options).expect("UF4 WAT");
    let wasm = emit_wasm_kir_module(&module, options).expect("UF4 WASM");
    (module, wat, wasm, minimum_trip)
}

fn run_node(wasm_o0: &[u8], wasm_o3: &[u8], runner: &str) {
    if !super::support::command::node_available() {
        eprintln!("skipping Wasm decision-tree runtime check: Node is unavailable");
        return;
    }
    let directory = super::support::temp::temp_dir("ck-wasm-decision-tree");
    fs::create_dir_all(&directory).expect("create temporary runtime directory");
    let o0_path = directory.join("baseline.wasm");
    let o3_path = directory.join("candidate.wasm");
    let runner_path = directory.join("runner.cjs");
    fs::write(&o0_path, wasm_o0).expect("write baseline module");
    fs::write(&o3_path, wasm_o3).expect("write candidate module");
    fs::write(&runner_path, runner).expect("write runtime oracle");
    let output = Command::new("node")
        .arg(&runner_path)
        .arg(&o0_path)
        .arg(&o3_path)
        .output()
        .expect("run Node oracle");
    let _ = fs::remove_dir_all(&directory);
    assert!(
        output.status.success(),
        "Node oracle failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn wasm_piecewise_simd_tree_should_match_strict_runtime_edges_and_keep_one_store_per_chunk() {
    let (baseline, _, baseline_wasm) = emit(PIECEWISE_SOURCE, KirOptimizationLevel::O0);
    let (candidate, wat, candidate_wasm) = emit(PIECEWISE_SOURCE, KirOptimizationLevel::O3);
    let function = candidate
        .functions
        .iter()
        .find(|function| function.name == "piecewise")
        .expect("piecewise function");
    assert_eq!(
        baseline.profile.digest_hex(),
        candidate.profile.digest_hex(),
        "runtime comparison uses the same target feature profile"
    );
    run_node(
        &baseline_wasm,
        &candidate_wasm,
        r#"
const fs = require("node:fs");
const [baselinePath, candidatePath] = process.argv.slice(2);
const baseline = new WebAssembly.Instance(new WebAssembly.Module(fs.readFileSync(baselinePath)));
const candidate = new WebAssembly.Instance(new WebAssembly.Module(fs.readFileSync(candidatePath)));
const values = [
  -0.25 - Number.EPSILON * 0.25, -0.25, -0.25 + Number.EPSILON * 0.25,
  -Number.MIN_VALUE, -0, +0, Number.MIN_VALUE,
  0.25 - Number.EPSILON * 0.25, 0.25, 0.25 + Number.EPSILON * 0.25,
  -Infinity, Infinity, Number.NaN
];
while (values.length < 64) values.push((values.length - 31) / 64);
function bits(view, address) { return view.getBigUint64(address, true); }
for (const instance of [baseline, candidate]) {
  const view = new DataView(instance.exports.memory.buffer);
  const input = 1024, output = 8192;
  values.forEach((value, index) => view.setFloat64(input + index * 8, value, true));
  instance.exports.piecewise(input, values.length, output, values.length);
}

const baseView = new DataView(baseline.exports.memory.buffer);
const candView = new DataView(candidate.exports.memory.buffer);
for (let index = 0; index < values.length; index += 1) {
  const address = 8192 + index * 8;
  const base = baseView.getFloat64(address, true);
  const actual = candView.getFloat64(address, true);
  if (Number.isNaN(base) && Number.isNaN(actual)) continue;
  if (bits(baseView, address) !== bits(candView, address)) {
    throw new Error(`lane ${index} differs: ${base} vs ${actual}`);
  }
}
function strictReference(x) {
  if (x < -0.25) { const square = x * x; return square + 0.5; }
  if (x < 0.0) { const scaled = x * 0.75; return scaled - 0.125; }
  if (x < 0.25) { const square = x * x; const cube = square * x; return cube + 0.25; }
  const shifted = x - 0.25; return shifted * 1.5;
}
function bitsOf(value) {
  const data = new DataView(new ArrayBuffer(8));
  data.setFloat64(0, value, true);
  return data.getBigUint64(0, true);
}
const trapInputs = [-0.5, -0.1, 0.1, 0.5];
for (const instance of [baseline, candidate]) {
  const memory = instance.exports.memory;
  const view = new DataView(memory.buffer);
  const input = 16384, output = memory.buffer.byteLength - 24;
  trapInputs.forEach((value, index) => view.setFloat64(input + index * 8, value, true));
  let trapped = false;
  try { instance.exports.piecewise(input, 4, output, 4); }
  catch (error) {
    if (!(error instanceof WebAssembly.RuntimeError)) throw error;
    trapped = true;
  }
  if (!trapped) throw new Error("physical output overrun did not trap");
  for (let index = 0; index < 3; index += 1) {
    const actual = view.getFloat64(output + index * 8, true);
    const expected = strictReference(trapInputs[index]);
    if (bits(view, output + index * 8) !== bitsOf(expected)) {
      throw new Error(`trap prefix lane ${index} differs: ${actual} vs ${expected}`);
    }
  }
}
"#,
    );
    let instructions = function
        .blocks
        .iter()
        .flat_map(|block| &block.instructions)
        .collect::<Vec<_>>();
    let load_starts = instructions
        .iter()
        .filter_map(|instruction| match &instruction.kind {
            calckernel::KirInstructionKind::VectorLoad { access, .. } => Some(access.start),
            _ => None,
        })
        .collect::<Vec<_>>();
    let store_starts = instructions
        .iter()
        .filter_map(|instruction| match &instruction.kind {
            calckernel::KirInstructionKind::VectorStore { access, .. } => Some(access.start),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        load_starts.len(),
        4,
        "UF4 emits one root load for each chunk"
    );
    assert_eq!(
        store_starts.len(),
        4,
        "UF4 emits one merged store for each chunk"
    );
    assert_eq!(
        load_starts, store_starts,
        "each chunk stores to the exact address it loaded"
    );
    assert_eq!(wat.matches("v128.load").count(), 4, "{wat}");
    assert_eq!(wat.matches("v128.store").count(), 4, "{wat}");
    assert!(
        wat.contains("v128.bitselect"),
        "nested vector selects are emitted"
    );
    wasmparser::Validator::new_with_features(
        wasmparser::WasmFeatures::MVP
            | wasmparser::WasmFeatures::MULTI_VALUE
            | wasmparser::WasmFeatures::BULK_MEMORY
            | wasmparser::WasmFeatures::SIMD,
    )
    .validate_all(&candidate_wasm)
    .expect("strict SIMD128 allowlist validates the candidate binary");
}

#[test]
fn wasm_piecewise_direct_uf4_matches_scalar_for_edges_tails_and_guard_fallback() {
    let (baseline, _, baseline_wasm) = emit(PIECEWISE_SOURCE, KirOptimizationLevel::O0);
    let (candidate, wat, candidate_wasm, minimum_trip) = emit_direct_uf4(PIECEWISE_SOURCE);
    assert_eq!(
        baseline.profile.digest_hex(),
        candidate.profile.digest_hex(),
        "runtime comparison uses the same strict SIMD128 target profile"
    );
    let wat_wasm = wat::parse_str(&wat).expect("UF4 WAT parses to a module");
    let function = candidate
        .functions
        .iter()
        .find(|function| function.name == "piecewise")
        .expect("piecewise function");
    let loads = function
        .blocks
        .iter()
        .flat_map(|block| &block.instructions)
        .filter(|instruction| {
            matches!(
                instruction.kind,
                calckernel::KirInstructionKind::VectorLoad { .. }
            )
        })
        .count();
    let stores = function
        .blocks
        .iter()
        .flat_map(|block| &block.instructions)
        .filter(|instruction| {
            matches!(
                instruction.kind,
                calckernel::KirInstructionKind::VectorStore { .. }
            )
        })
        .count();
    assert_eq!(
        loads, 4,
        "four exact contiguous input chunks are materialized"
    );
    assert_eq!(stores, 4, "four ordered output chunks are materialized");
    assert!(minimum_trip >= 8 && minimum_trip.is_multiple_of(8));
    assert!(
        !wat.contains("relaxed_") && !wat.contains("fma"),
        "UF4 remains strict SIMD: {wat}"
    );
    for module_bytes in [&candidate_wasm, &wat_wasm] {
        wasmparser::Validator::new_with_features(
            wasmparser::WasmFeatures::MVP
                | wasmparser::WasmFeatures::MULTI_VALUE
                | wasmparser::WasmFeatures::BULK_MEMORY
                | wasmparser::WasmFeatures::SIMD,
        )
        .validate_all(module_bytes)
        .expect("strict SIMD128 allowlist validates the UF4 module");
    }

    run_node(
        &baseline_wasm,
        &candidate_wasm,
        &format!(
            r#"
const fs = require("node:fs");
const [baselinePath, candidatePath] = process.argv.slice(2);
const baseline = new WebAssembly.Instance(new WebAssembly.Module(fs.readFileSync(baselinePath)));
const candidate = new WebAssembly.Instance(new WebAssembly.Module(fs.readFileSync(candidatePath)));
const edgeValues = [-0.25, -0.25 + Number.EPSILON, -Number.MIN_VALUE, -0, +0, Number.MIN_VALUE,
  0.25 - Number.EPSILON, 0.25, Infinity, -Infinity, Number.NaN];
function bits(view, address) {{ return view.getBigUint64(address, true); }}
const lengths = [0, 1, 7, 8, 9, {minimum_trip}, {minimum_trip}+1, {minimum_trip}+7, {minimum_trip}+8, 129];
for (const length of lengths) {{
  for (const instance of [baseline, candidate]) {{
    const view = new DataView(instance.exports.memory.buffer);
    const input = 1024, output = 8192, canary = 0x5a5a5a5a5a5a5a5an;
    for (let index = 0; index < length; index += 1) {{
      const value = edgeValues[index % edgeValues.length] ?? ((index % 51) - 25) / 13;
      view.setFloat64(input + index * 8, value, true);
    }}
    view.setBigUint64(output + length * 8, canary, true);
    instance.exports.piecewise(input, length, output, length);
    if (view.getBigUint64(output + length * 8, true) !== canary) throw new Error("length " + length + " overwrote output canary");
  }}
  const base = new DataView(baseline.exports.memory.buffer);
  const actual = new DataView(candidate.exports.memory.buffer);
  for (let index = 0; index < length; index += 1) {{
    const address = 8192 + index * 8;
    const left = base.getFloat64(address, true), right = actual.getFloat64(address, true);
    if (Number.isNaN(left) && Number.isNaN(right)) continue;
    if (bits(base, address) !== bits(actual, address)) throw new Error("length " + length + " lane " + index + " differs: " + left + " / " + right);
  }}
}}
const trapLength = {minimum_trip};
function strictReference(x) {{
  if (x < -0.25) {{ const square = x * x; return square + 0.5; }}
  if (x < 0.0) {{ const scaled = x * 0.75; return scaled - 0.125; }}
  if (x < 0.25) {{ const square = x * x; const cube = square * x; return cube + 0.25; }}
  const shifted = x - 0.25; return shifted * 1.5;
}}
const trappedViews = [];
for (const instance of [baseline, candidate]) {{
  const memory = instance.exports.memory;
  const view = new DataView(memory.buffer);
  const input = 1024, output = memory.buffer.byteLength - (trapLength - 1) * 8;
  for (let index = 0; index < trapLength; index += 1) {{
    view.setFloat64(input + index * 8, edgeValues[index % edgeValues.length] ?? ((index % 51) - 25) / 13, true);
  }}
  let trapped = false;
  try {{ instance.exports.piecewise(input, trapLength, output, trapLength); }}
  catch (error) {{
    if (!(error instanceof WebAssembly.RuntimeError)) throw error;
    trapped = true;
  }}
  if (!trapped) throw new Error("out-of-bounds UF4 guard case did not trap");
  trappedViews.push({{ view, output }});
}}
const bitsOf = value => {{
  const storage = new DataView(new ArrayBuffer(8));
  storage.setFloat64(0, value, true);
  return storage.getBigUint64(0, true);
}};
for (let index = 0; index < trapLength - 1; index += 1) {{
  const expected = strictReference(edgeValues[index % edgeValues.length] ?? ((index % 51) - 25) / 13);
  for (const {{ view, output }} of trappedViews) {{
    const actual = view.getFloat64(output + index * 8, true);
    if (Number.isNaN(expected) && Number.isNaN(actual)) continue;
    if (view.getBigUint64(output + index * 8, true) !== bitsOf(expected)) {{
      throw new Error("trap prefix lane " + index + " changed before the guard fallback trap");
    }}
  }}
}}
"#
        ),
    );
}
