use calckernel::{
    EmitWasmOptions, KirBuildConfig, KirConsumer, KirOptimizationLevel, KirOverflowMode,
    KirSanitizerMode, KirTargetProfile, KirVerifiedProgramState, KirWasmFeatures, SourceFile,
    build_kir_module_with_profile, check, check_vectorization_trial_independently,
    discover_vectorization_candidates, emit_wasm_kir_module, emit_wat_kir_module,
    import_contract_facts, lower_to_mir, prepare_vectorization_trial, run_kir_pass_pipeline,
    validate_kir_optimization_evidence,
};

const AFFINE_SOURCE: &str = r#"
export unsafe fn affine_update(
  a: slice<f64>, b: slice<f64>, out: slice<f64>, n: u32,
  a_index: u32, b_offset: u32, out_offset: u32
) -> void contract {
  requires noalias(a, out);
  requires noalias(b, out);
  effects read(a), read(b), readwrite(out);
} {
  let i: u32 = 0;
  while i < n {
    let out_index: u32 = out_offset + i;
    let b_index: u32 = b_offset + i;
    let previous: f64 = out[out_index];
    let broadcast: f64 = a[a_index];
    let contiguous: f64 = b[b_index];
    out[out_index] = previous + broadcast * contiguous;
    i = i + 1;
  }
}
"#;

// Keep the full nested matrix source contract in the public regression test.
// A host's n*n allocation promise is not a fact available to the compiler.
const FROZEN_MATMUL_SOURCE: &str = r#"
export unsafe fn ck_bench_matmul(
  a: slice<f64>,
  b: slice<f64>,
  out: slice<f64>,
  n: u32
) -> void contract {
  requires n != 0;
  requires n <= a.len && n <= b.len && n <= out.len;
  requires a.len == b.len && b.len == out.len;
  requires noalias(a, b);
  requires noalias(a, out);
  requires noalias(b, out);
  effects read(a), read(b), readwrite(out);
} {
  let i: u32 = 0;
  while i < out.len {
    out[i] = 0.0;
    i = i + 1;
  }

  let row: u32 = 0;
  while row < n {
    let inner: u32 = 0;
    while inner < n {
      let col: u32 = 0;
      while col < n {
        let out_index: u32 = row * n + col;
        let a_index: u32 = row * n + inner;
        let b_index: u32 = inner * n + col;
        out[out_index] = out[out_index] + a[a_index] * b[b_index];
        col = col + 1;
      }
      inner = inner + 1;
    }
    row = row + 1;
  }
}
"#;

fn wasm_state(source: &str, level: KirOptimizationLevel) -> KirVerifiedProgramState {
    let checked = check(&SourceFile::new("wasm-affine.ck", source));
    assert_eq!(checked.diagnostics, [], "invalid fixture:\n{source}");
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
    .expect("WASM KIR");
    let contracts =
        import_contract_facts(&module, &checked.checked_program, 0).expect("contract facts");
    let optimized = run_kir_pass_pipeline(module, level, Some(&contracts));
    assert!(optimized.errors.is_empty(), "{:?}", optimized.errors);
    KirVerifiedProgramState::from_parts(
        optimized.artifact.expect("optimized pre-state"),
        optimized.contract_facts,
        optimized.proofs,
        optimized.eliminated_guards,
        0,
    )
    .expect("verified pre-state")
}

fn simd128_allowlist() -> wasmparser::WasmFeatures {
    wasmparser::WasmFeatures::MVP
        | wasmparser::WasmFeatures::MULTI_VALUE
        | wasmparser::WasmFeatures::BULK_MEMORY
        | wasmparser::WasmFeatures::SIMD
}

fn materialize_affine_trial(
    source: &str,
    level: KirOptimizationLevel,
    function_name: &str,
) -> (KirVerifiedProgramState, calckernel::PreparedVectorization) {
    let state = wasm_state(source, level);
    let function = state
        .module()
        .functions
        .iter()
        .find(|function| function.name == function_name)
        .expect("candidate function");
    let candidate = discover_vectorization_candidates(&state)
        .candidates
        .into_iter()
        .find(|candidate| candidate.function == function.id && candidate.wasm_affine.is_some())
        .unwrap_or_else(|| panic!("no affine candidate for {function_name}"));
    assert_eq!((candidate.vf, candidate.uf), (2, 1));
    let prepared = prepare_vectorization_trial(&state, &candidate).expect("materialized trial");
    let evidence = validate_kir_optimization_evidence(
        prepared.trial.module(),
        prepared.trial.contract_facts(),
        prepared.trial.proofs(),
        prepared.trial.eliminated_guards(),
        prepared.trial.evidence_generation(),
    );
    assert!(
        evidence.errors.is_empty(),
        "KIR validation: {:?}",
        evidence.errors
    );
    (state, prepared)
}

fn direct_affine_trial(
    source: &str,
    level: KirOptimizationLevel,
    function_name: &str,
) -> (KirVerifiedProgramState, calckernel::PreparedVectorization) {
    let (state, prepared) = materialize_affine_trial(source, level, function_name);
    let checked = check_vectorization_trial_independently(
        &state,
        &prepared.trial,
        &prepared.plan,
        &prepared.charge,
    );
    assert!(checked.is_ok(), "independent checker: {checked:?}");
    (state, prepared)
}

#[test]
fn wasm_affine_direct_trial_emits_wat_and_binary_after_independent_check() {
    let (state, prepared) =
        direct_affine_trial(AFFINE_SOURCE, KirOptimizationLevel::O2, "affine_update");
    let module = prepared.trial.module();
    let function = module
        .functions
        .iter()
        .find(|function| function.name == "affine_update")
        .expect("affine_update");
    let vector_body = function
        .blocks
        .iter()
        .find(|block| block.label == "loop_simd_body")
        .expect("materialized vector body");
    assert_eq!(prepared.plan.broadcast_groups.len(), 1);
    assert_eq!(
        vector_body
            .instructions
            .iter()
            .filter(|instruction| matches!(
                instruction.kind,
                calckernel::KirInstructionKind::VectorLoad { .. }
                    | calckernel::KirInstructionKind::VectorStore { .. }
            ))
            .count(),
        3
    );

    let options = EmitWasmOptions { opt_level: 3 };
    let wat = emit_wat_kir_module(module, options).expect("affine trial WAT");
    assert!(wat.contains("v128.load"), "{wat}");
    assert!(wat.contains("v128.store"), "{wat}");
    let wasm = emit_wasm_kir_module(module, options).expect("affine trial direct binary");
    wasmparser::Validator::new_with_features(simd128_allowlist())
        .validate_all(&wasm)
        .expect("direct binary validates with the exact SIMD128 allowlist");
    assert_eq!(
        state.module().profile.wasm_features(),
        Some(KirWasmFeatures::Simd128)
    );
}

#[test]
fn frozen_matmul_o3_pipeline_emits_checked_affine_wat_and_binary() {
    let optimized = wasm_state(FROZEN_MATMUL_SOURCE, KirOptimizationLevel::O3);
    let function = optimized
        .module()
        .functions
        .iter()
        .find(|function| function.name == "ck_bench_matmul")
        .expect("matmul function");
    assert_eq!(
        function.vector_regions.len(),
        1,
        "O3 accepts one checked SIMD region"
    );
    let body = function
        .blocks
        .iter()
        .find(|block| block.label == "loop_simd_body")
        .expect("matmul vector body");
    assert_eq!(
        body.instructions
            .iter()
            .filter(|instruction| matches!(
                instruction.kind,
                calckernel::KirInstructionKind::VectorLoad { .. }
                    | calckernel::KirInstructionKind::VectorStore { .. }
            ))
            .count(),
        12,
        "UF4 has one output and one B vector access per unrolled chunk"
    );
    assert_eq!(
        body.instructions
            .iter()
            .filter(|instruction| matches!(
                instruction.kind,
                calckernel::KirInstructionKind::Load { .. }
            ))
            .count(),
        1,
        "the invariant A element is loaded once for the UF4 bundle"
    );
    assert_eq!(
        body.instructions
            .iter()
            .filter(|instruction| matches!(
                instruction.kind,
                calckernel::KirInstructionKind::VectorSplat { .. }
            ))
            .count(),
        1,
        "the shared A element is splatted once"
    );
    assert_eq!(
        body.instructions
            .iter()
            .filter(|instruction| matches!(
                instruction.kind,
                calckernel::KirInstructionKind::VectorBinary {
                    op: calckernel::KirVectorBinaryOp::Multiply,
                    ..
                }
            ))
            .count(),
        4
    );
    assert_eq!(
        body.instructions
            .iter()
            .filter(|instruction| matches!(
                instruction.kind,
                calckernel::KirInstructionKind::VectorBinary {
                    op: calckernel::KirVectorBinaryOp::Add,
                    ..
                }
            ))
            .count(),
        4
    );
    let options = EmitWasmOptions { opt_level: 3 };
    let wat = emit_wat_kir_module(optimized.module(), options).expect("matmul O3 WAT");
    assert!(wat.contains("v128.load"), "{wat}");
    assert!(wat.contains("v128.store"), "{wat}");
    let wasm = emit_wasm_kir_module(optimized.module(), options).expect("matmul O3 binary");
    wasmparser::Validator::new_with_features(simd128_allowlist())
        .validate_all(&wasm)
        .expect("matmul O3 binary validates with the exact SIMD128 allowlist");
}

#[test]
fn wasm_affine_direct_trial_preserves_strict_outputs_alias_and_trap_prefix() {
    if !crate::support::command::node_available() {
        return;
    }
    let (state, prepared) =
        direct_affine_trial(AFFINE_SOURCE, KirOptimizationLevel::O2, "affine_update");
    let options = EmitWasmOptions { opt_level: 3 };
    let binaries = [state.module(), prepared.trial.module()]
        .map(|module| emit_wasm_kir_module(module, options).expect("affine binary"));
    let script = r#"
      const assert = require('node:assert/strict');
      const binaries = JSON.parse(process.argv[1]);
      const patterns = [0n, 0x8000000000000000n, 1n,
        0x3ff4000000000000n, 0xc004000000000000n,
        0x7ff0000000000000n, 0xfff0000000000000n];
      function run(binary, n, aIndex, bOffset, outOffset, length, aliasAB, nearEnd) {
        const instance = new WebAssembly.Instance(
          new WebAssembly.Module(Uint8Array.from(binary)));
        const { memory, affine_update } = instance.exports;
        const raw = new Uint8Array(memory.buffer);
        raw.fill(0xa5);
        const view = new DataView(memory.buffer);
        const a = 1024, b = aliasAB ? a : 4096;
        const out = nearEnd ? raw.length - 16 : 8192;
        for (let i = 0; i < 32; i++) {
          view.setBigUint64(a + i * 8, patterns[i % patterns.length], true);
          view.setBigUint64(b + i * 8, patterns[(i + 3) % patterns.length], true);
          if (!nearEnd) view.setBigUint64(out + i * 8,
            patterns[(i + 1) % patterns.length], true);
        }
        let trapped = false;
        try {
          affine_update(a, length, b, length, out, nearEnd ? 2 : length,
            n, aIndex, bOffset, outOffset);
        } catch (error) {
          assert(error instanceof WebAssembly.RuntimeError);
          trapped = true;
        }
        return { trapped, bytes: Buffer.from(raw) };
      }
      const cases = [
        [0,0,0,0,0,false,false], [1,0,0,0,1,false,false],
        [2,1,0,0,2,false,false], [3,2,1,1,4,false,false],
        [9,3,2,3,16,false,false], [17,4,0,0,32,true,false],
        [9,1,25,25,8,false,false], [4,1,0,0,32,false,true]
      ];
      for (const args of cases) {
        const scalar = run(binaries[0], ...args);
        const vector = run(binaries[1], ...args);
        assert.equal(vector.trapped, scalar.trapped, String(args));
        assert(vector.bytes.equals(scalar.bytes), `byte mismatch: ${args}`);
      }
    "#;
    let output = std::process::Command::new("node")
        .args(["-e", script, &serde_json::to_string(&binaries).unwrap()])
        .output()
        .expect("Node affine oracle");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn frozen_matmul_o3_pipeline_preserves_nested_strict_order_and_scalar_traps() {
    if !crate::support::command::node_available() {
        return;
    }
    let scalar = wasm_state(FROZEN_MATMUL_SOURCE, KirOptimizationLevel::O2);
    let optimized = wasm_state(FROZEN_MATMUL_SOURCE, KirOptimizationLevel::O3);
    let binaries = [scalar.module(), optimized.module()].map(|module| {
        emit_wasm_kir_module(module, EmitWasmOptions { opt_level: 3 }).expect("matmul binary")
    });
    let script = r#"
      const assert = require('node:assert/strict');
      const binaries = JSON.parse(process.argv[1]);
      const patterns = [0n, 0x8000000000000000n, 1n,
        0x3ff0000000000000n, 0xbff0000000000000n,
        0x3ff4000000000000n, 0xc004000000000000n];
      function run(binary, n, length, nearEnd) {
        const instance = new WebAssembly.Instance(
          new WebAssembly.Module(Uint8Array.from(binary)));
        const { memory, ck_bench_matmul } = instance.exports;
        const raw = new Uint8Array(memory.buffer);
        raw.fill(0xa5);
        const view = new DataView(memory.buffer);
        const a = 1024, b = 4096, out = nearEnd ? raw.length - length * 8 : 8192;
        for (let i = 0; i < length; i++) {
          view.setBigUint64(a + i * 8, patterns[i % patterns.length], true);
          view.setBigUint64(b + i * 8, patterns[(i + 3) % patterns.length], true);
          view.setBigUint64(out + i * 8, patterns[(i + 1) % patterns.length], true);
        }
        let trapped = false;
        try { ck_bench_matmul(a, length, b, length, out, length, n); }
        catch (error) {
          assert(error instanceof WebAssembly.RuntimeError);
          trapped = true;
        }
        return { trapped, bytes: Buffer.from(raw) };
      }
      for (const [n,length,nearEnd] of [
        [1,1,false], [2,4,false], [3,9,false], [4,16,false],
        [5,25,false], [4,4,true]
      ]) {
        const scalar = run(binaries[0], n, length, nearEnd);
        const vector = run(binaries[1], n, length, nearEnd);
        assert.equal(vector.trapped, scalar.trapped, `${n}/${length}/${nearEnd}`);
        assert(vector.bytes.equals(scalar.bytes),
          `byte mismatch: ${n}/${length}/${nearEnd}`);
      }
    "#;
    let output = std::process::Command::new("node")
        .args(["-e", script, &serde_json::to_string(&binaries).unwrap()])
        .output()
        .expect("Node matmul oracle");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
