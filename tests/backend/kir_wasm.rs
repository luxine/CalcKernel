use std::{fs, process::Command};

use calckernel::{
    BlockId, EmitWasmOptions, FunctionId, InstructionId, KirBlock, KirBoundsMode, KirBuildConfig,
    KirConsumer, KirEdge, KirFunction, KirInstruction, KirInstructionKind, KirOptimizationLevel,
    KirOverflowMode, KirParam, KirResult, KirSanitizerMode, KirTargetProfile, KirTerminator,
    KirValueType, KirVersionPredicate, KirVersionPredicateConjunct, KirWasmFeatures,
    MirPrimitiveTypeName, MirType, SourceFile, ValueId, build_kir_module,
    build_kir_module_with_profile, check, emit_wasm_kir_module, emit_wat_kir_module,
    import_contract_facts, lower_to_mir, run_kir_pass_pipeline,
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

fn wasm_features(features: KirWasmFeatures) -> wasmparser::WasmFeatures {
    let mut allowed = wasmparser::WasmFeatures::MVP | wasmparser::WasmFeatures::MULTI_VALUE;
    if features == KirWasmFeatures::Simd128 {
        allowed |= wasmparser::WasmFeatures::SIMD;
    }
    allowed
}

fn optimized_simd128_kir(
    source: &str,
    features: KirWasmFeatures,
    level: KirOptimizationLevel,
) -> calckernel::KirPassManagerResult {
    let checked = check(&SourceFile::new("kir-wasm-simd.ck", source));
    assert_eq!(checked.diagnostics, []);
    let mir = lower_to_mir(&checked.checked_program).expect("MIR");
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
    .expect("profiled WebAssembly KIR");
    let contracts =
        import_contract_facts(&module, &checked.checked_program, 0).expect("map contract facts");
    let result = run_kir_pass_pipeline(module, level, Some(&contracts));
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    result
}

fn pure_diamond_source(lane: &str, comparison: &str) -> String {
    format!(
        r#"
export unsafe fn map(a: slice<{lane}>, b: slice<{lane}>, n: u32,
                    pivot: {lane}, delta: {lane}) -> void
contract {{ requires n <= a.len && n <= b.len; requires noalias(a, b); effects read(a), write(b); }}
{{
  let i: u32 = 0;
  while i < n {{
    let x: {lane} = a[i];
    let selected: {lane} = delta;
    if x {comparison} pivot {{ selected = x + delta; }} else {{ selected = x - delta; }}
    b[i] = selected;
    i = i + 1;
  }}
}}
"#
    )
}

fn compare_select_runner(lane: &str, comparison: &str, lanes: u8) -> String {
    let (bytes, values, pivot, delta, set, get, expected) = match lane {
        "f64" => (
            8,
            "[NaN, -Infinity, -Number.MAX_VALUE, -1, -0, 0, 1, Number.MAX_VALUE, Infinity, Number.MIN_VALUE, -Number.MIN_VALUE]",
            "0",
            "0.5",
            "setFloat64",
            "getFloat64",
            "condition ? x + delta : x - delta",
        ),
        "i32" => (
            4,
            "[-0x80000000, -0x7fffffff, -2, -1, 0, 1, 2, 0x7ffffffe, 0x7fffffff]",
            "0",
            "3",
            "setInt32",
            "getInt32",
            "condition ? ((x + delta) | 0) : ((x - delta) | 0)",
        ),
        "u32" => (
            4,
            "[0, 1, 2, 0x7fffffff, 0x80000000, 0x80000001, 0xfffffffe, 0xffffffff]",
            "0x80000000",
            "3",
            "setUint32",
            "getUint32",
            "condition ? ((x + delta) >>> 0) : ((x - delta) >>> 0)",
        ),
        _ => unreachable!("known comparison lane"),
    };
    format!(
        r#"
import fs from "node:fs";
import assert from "node:assert/strict";
const {{ instance }} = await WebAssembly.instantiate(fs.readFileSync(process.argv[2]), {{}});
const view = new DataView(instance.exports.memory.buffer);
const input = {input};
const output = {output};
const guardBefore = 12345;
const guardAfter = 23456;
const values = {values};
const pivot = {pivot};
const delta = {delta};
const sizes = [0, 1, {lanes_minus_one}, {lanes}, {lanes_plus_one}, {two_lanes_plus_one}, 1024, 1025];
for (const size of sizes) {{
  view.{set}(output - {bytes}, guardBefore, true);
  view.{set}(output + size * {bytes}, guardAfter, true);
  for (let i = 0; i < size; i++) view.{set}(input + i * {bytes}, values[i % values.length], true);
  instance.exports.map(input, size, output, size, size, pivot, delta);
  for (let i = 0; i < size; i++) {{
    const x = values[i % values.length];
    const condition = x {comparison} pivot;
    const expected = {expected};
    assert.ok(Object.is(view.{get}(output + i * {bytes}, true), expected),
      `{lane} {comparison} size=${{size}} index=${{i}}`);
  }}
  assert.equal(view.{get}(output - {bytes}, true), guardBefore, "left sentinel");
  assert.equal(view.{get}(output + size * {bytes}, true), guardAfter, "right sentinel");
}}
"#,
        input = if bytes == 8 { 264 } else { 260 },
        output = if bytes == 8 { 49160 } else { 49156 },
        lanes_minus_one = lanes - 1,
        lanes = lanes,
        lanes_plus_one = lanes + 1,
        two_lanes_plus_one = 2 * lanes + 1,
    )
}

fn simd_compare_opcode(lane: &str, comparison: &str) -> String {
    let name = match comparison {
        "==" => "eq",
        "!=" => "ne",
        "<" => "lt",
        "<=" => "le",
        ">" => "gt",
        ">=" => "ge",
        _ => unreachable!("known comparison"),
    };
    let lane_type = match lane {
        "f64" => "f64x2",
        "i32" => "i32x4",
        "u32" => "i32x4",
        _ => unreachable!("known comparison lane"),
    };
    match (lane, comparison) {
        ("i32", "<" | "<=" | ">" | ">=") => format!("{lane_type}.{name}_s"),
        ("u32", "<" | "<=" | ">" | ">=") => format!("{lane_type}.{name}_u"),
        _ => format!("{lane_type}.{name}"),
    }
}

#[test]
fn kir_wasm_backend_should_emit_a_single_block_self_loop() {
    let mut kir = optimized_kir("export fn spin() -> void {}", KirOptimizationLevel::O0);
    let function = kir
        .functions
        .iter_mut()
        .find(|function| function.name == "spin")
        .expect("spin function");
    assert_eq!(function.blocks.len(), 1, "expected a single-block loop");
    let block = &mut function.blocks[0];
    block.terminator = KirTerminator::Jump {
        edge: KirEdge {
            target: block.id,
            args: Vec::new(),
            memory_args: Vec::new(),
        },
    };
    assert_eq!(calckernel::validate_kir_module(&kir).errors, []);

    let options = EmitWasmOptions { opt_level: 3 };
    let wat = emit_wat_kir_module(&kir, options).expect("single-block loop WAT");
    let wasm = emit_wasm_kir_module(&kir, options).expect("single-block loop WASM");
    assert!(wat.contains("(export \"spin\")"));
    assert_eq!(&wasm[..8], b"\0asm\x01\0\0\0");
}

#[test]
fn kir_wasm_backend_rejects_targetless_vector_store_without_panicking() {
    use calckernel::{
        InstructionId, KirInstruction, KirLaneType, KirVectorMemoryAccess, VectorRegionId,
    };

    let mut kir = optimized_kir(
        "export fn scalar(items: slice<i32>, index: u32) -> i32 { return items[index]; }",
        KirOptimizationLevel::O0,
    );
    let function = kir
        .functions
        .iter_mut()
        .find(|function| function.name == "scalar")
        .expect("scalar function");
    let slice = function.params[0].value;
    let index = function.params[1].value;
    let block = function.blocks.first_mut().expect("entry block");
    block.instructions.push(KirInstruction {
        id: InstructionId::from_index(u32::MAX),
        results: Vec::new(),
        kind: KirInstructionKind::VectorStore {
            access: KirVectorMemoryAccess {
                slice,
                start: index,
                end: index,
                lane: KirLaneType::I32,
                lanes: 4,
                byte_footprint: 16,
                known_alignment: 4,
                required_alignment: 4,
            },
            value: index,
            region: VectorRegionId::from_index(u32::MAX),
        },
        memory: None,
        effect: None,
    });

    let result =
        std::panic::catch_unwind(|| emit_wat_kir_module(&kir, EmitWasmOptions { opt_level: 3 }));
    let emitted = result.expect("vector KIR must be rejected without panic");
    assert!(emitted.is_err(), "targetless VectorStore must fail closed");
}

#[test]
fn kir_wasm_backend_should_reject_baseline_and_unsupported_vector_kir_without_panicking() {
    const F64_MAP: &str = include_str!("../../examples/wasm/f64_map.ck");

    let optimized =
        optimized_simd128_kir(F64_MAP, KirWasmFeatures::Simd128, KirOptimizationLevel::O3);
    assert_eq!(optimized.stats.vectorized_loops, 1);
    let mut baseline = optimized.artifact.expect("verified vector KIR");
    baseline.profile = KirTargetProfile::webassembly_with_features(KirWasmFeatures::Baseline);
    let baseline_result = std::panic::catch_unwind(|| {
        emit_wat_kir_module(&baseline, EmitWasmOptions { opt_level: 3 })
    });
    assert!(
        baseline_result.is_ok(),
        "baseline vector rejection must not panic"
    );
    assert!(
        baseline_result.unwrap().is_err(),
        "baseline must reject vector KIR"
    );

    let optimized =
        optimized_simd128_kir(F64_MAP, KirWasmFeatures::Simd128, KirOptimizationLevel::O3);
    let mut unsupported = optimized.artifact.expect("verified vector KIR");
    let vector_binary = unsupported
        .functions
        .iter_mut()
        .flat_map(|function| &mut function.blocks)
        .flat_map(|block| &mut block.instructions)
        .find_map(|instruction| match &mut instruction.kind {
            KirInstructionKind::VectorBinary { op, .. }
                if *op == calckernel::KirVectorBinaryOp::Multiply =>
            {
                Some(op)
            }
            _ => None,
        })
        .expect("vector multiply");
    *vector_binary = calckernel::KirVectorBinaryOp::Remainder;
    let unsupported_result = std::panic::catch_unwind(|| {
        emit_wat_kir_module(&unsupported, EmitWasmOptions { opt_level: 3 })
    });
    assert!(
        unsupported_result.is_ok(),
        "unsupported vector rejection must not panic"
    );
    assert!(
        unsupported_result.unwrap().is_err(),
        "unsupported remainder must fail closed"
    );

    let optimized =
        optimized_simd128_kir(F64_MAP, KirWasmFeatures::Simd128, KirOptimizationLevel::O3);
    let mut malformed = optimized.artifact.expect("verified vector KIR");
    let vector_load = malformed
        .functions
        .iter_mut()
        .flat_map(|function| &mut function.blocks)
        .flat_map(|block| &mut block.instructions)
        .find_map(|instruction| match &mut instruction.kind {
            KirInstructionKind::VectorLoad { access, .. } => Some(access),
            _ => None,
        })
        .expect("vector load");
    vector_load.lanes = 4;
    let malformed_result = std::panic::catch_unwind(|| {
        emit_wat_kir_module(&malformed, EmitWasmOptions { opt_level: 3 })
    });
    assert!(
        malformed_result.is_ok(),
        "malformed vector shape must not panic"
    );
    assert!(
        malformed_result.unwrap().is_err(),
        "lane mismatch must fail closed"
    );

    for mutation in 0..5 {
        let optimized =
            optimized_simd128_kir(F64_MAP, KirWasmFeatures::Simd128, KirOptimizationLevel::O3);
        let mut malformed = optimized.artifact.expect("verified vector KIR");
        let access = malformed
            .functions
            .iter_mut()
            .flat_map(|function| &mut function.blocks)
            .flat_map(|block| &mut block.instructions)
            .find_map(|instruction| match (&mut instruction.kind, mutation) {
                (KirInstructionKind::VectorLoad { access, .. }, 0..=2) => Some(access),
                (KirInstructionKind::VectorStore { access, .. }, 3..=4) => Some(access),
                _ => None,
            })
            .expect("matching vector access");
        match mutation {
            0 => access.byte_footprint = 8,
            1 => access.required_alignment = 16,
            2 => access.known_alignment = 4,
            3 => access.required_alignment = 16,
            4 => access.byte_footprint = 8,
            _ => unreachable!(),
        }
        let malformed_result = std::panic::catch_unwind(|| {
            emit_wat_kir_module(&malformed, EmitWasmOptions { opt_level: 3 })
        });
        assert!(
            malformed_result.is_ok(),
            "invalid access {mutation} must not panic"
        );
        assert!(
            malformed_result.unwrap().is_err(),
            "invalid access {mutation} must fail closed"
        );
    }
}

#[test]
fn kir_wasm_simd128_maps_should_match_scalar_at_lane_boundaries_and_natural_alignment() {
    const F64_MAP: &str = include_str!("../../examples/wasm/f64_map.ck");
    const I32_MAP: &str = include_str!("../../examples/wasm/i32_map.ck");
    const F64_RUNNER: &str = r#"
        import fs from "node:fs";
        const { instance } = await WebAssembly.instantiate(fs.readFileSync(process.argv[2]), {});
        const view = new DataView(instance.exports.memory.buffer);
        const input = 264;
        const output = 49160;
        const guard = 123456.5;
        const values = [1 + 2 ** -27, -0, 0, NaN, Infinity, -Infinity,
          Number.MIN_VALUE, -Number.MIN_VALUE, Number.MAX_VALUE, -Number.MAX_VALUE,
          0.5, -0.5, 1.000000000000001];
        const sizes = [0, 1, 2, 3, 4, 5, 7, 8, 9, 15, 16, 17, 31, 32, 33, 1024, 1025];
        const parameters = [[1, -0], [1 - 2 ** -27, -1], [Infinity, 0]];
        for (const size of sizes) for (const [factor, bias] of parameters) {
          view.setFloat64(output - 8, guard, true);
          view.setFloat64(output + size * 8, guard, true);
          for (let i = 0; i < size; ++i) view.setFloat64(input + i * 8, values[i % values.length], true);
          instance.exports.map_f64(input, size, output, size, size, factor, bias);
          for (let i = 0; i < size; ++i) {
            const expected = values[i % values.length] * factor + bias;
            if (!Object.is(view.getFloat64(output + i * 8, true), expected)) process.exit(10);
          }
          if (!Object.is(view.getFloat64(output - 8, true), guard) ||
              !Object.is(view.getFloat64(output + size * 8, true), guard)) process.exit(11);
        }
    "#;
    const I32_RUNNER: &str = r#"
        import fs from "node:fs";
        const { instance } = await WebAssembly.instantiate(fs.readFileSync(process.argv[2]), {});
        const view = new DataView(instance.exports.memory.buffer);
        const input = 260;
        const output = 49156;
        const guard = 0x13579bdf;
        const factor = 0x6f3a2b17;
        const bias = 0x6123bcde;
        const values = [0x7fffffff, -0x80000000, -1, 0, 0x40000001,
          -0x40000001, 0x12345678, -0x12345678, 0x6fffffff, -0x70000000,
          17, -23, 1, -1, 0x55555555, -0x55555556];
        const sizes = [0, 1, 2, 3, 4, 5, 7, 8, 9, 15, 16, 17, 31, 32, 33, 1024, 1025];
        for (const size of sizes) {
          view.setInt32(output - 4, guard, true);
          view.setInt32(output + size * 4, guard, true);
          for (let i = 0; i < size; ++i) view.setInt32(input + i * 4, values[i % values.length], true);
          instance.exports.map_i32(input, size, output, size, size, factor, bias);
          for (let i = 0; i < size; ++i) {
            const expected = (Math.imul(values[i % values.length], factor) + bias) | 0;
            if (view.getInt32(output + i * 4, true) !== expected) process.exit(20);
          }
          if (view.getInt32(output - 4, true) !== guard ||
              view.getInt32(output + size * 4, true) !== guard) process.exit(21);
        }
    "#;

    for case in [
        (
            F64_MAP,
            KirWasmFeatures::Baseline,
            KirOptimizationLevel::O3,
            3,
            F64_RUNNER,
            0,
        ),
        (
            F64_MAP,
            KirWasmFeatures::Simd128,
            KirOptimizationLevel::O0,
            0,
            F64_RUNNER,
            0,
        ),
        (
            F64_MAP,
            KirWasmFeatures::Simd128,
            KirOptimizationLevel::O3,
            3,
            F64_RUNNER,
            1,
        ),
        (
            F64_MAP,
            KirWasmFeatures::Simd128,
            KirOptimizationLevel::O3,
            0,
            F64_RUNNER,
            1,
        ),
        (
            I32_MAP,
            KirWasmFeatures::Baseline,
            KirOptimizationLevel::O3,
            3,
            I32_RUNNER,
            0,
        ),
        (
            I32_MAP,
            KirWasmFeatures::Simd128,
            KirOptimizationLevel::O0,
            0,
            I32_RUNNER,
            0,
        ),
        (
            I32_MAP,
            KirWasmFeatures::Simd128,
            KirOptimizationLevel::O3,
            3,
            I32_RUNNER,
            1,
        ),
        (
            I32_MAP,
            KirWasmFeatures::Simd128,
            KirOptimizationLevel::O3,
            0,
            I32_RUNNER,
            1,
        ),
    ] {
        let optimized = optimized_simd128_kir(case.0, case.1, case.2);
        assert_eq!(optimized.stats.vectorized_loops, case.5);
        let module = optimized.artifact.as_ref().expect("verified KIR");
        let options = EmitWasmOptions { opt_level: case.3 };
        let wat = emit_wat_kir_module(module, options).expect("map WAT artifact");
        if case.5 == 1 && case.3 == 0 {
            assert!(
                wat.contains("br_table"),
                "low-opt vector KIR must use typed dispatcher: {wat}"
            );
            assert!(
                wat.contains("v128.load") && wat.contains("v128.store"),
                "{wat}"
            );
        }
        let wasm = emit_wasm_kir_module(module, options).expect("map Wasm artifact");
        wasmparser::Validator::new_with_features(wasm_features(case.1))
            .validate_all(&wasm)
            .expect("map Wasm validates against its target profile");
        run_wasm(&wasm, case.4);
    }
}

#[test]
fn kir_wasm_simd128_u32_map_should_emit_i32x4_and_run_modular_lanes() {
    const SOURCE: &str = r#"
        export unsafe fn map_u32(a: slice<u32>, b: slice<u32>, n: u32,
                                 factor: u32, bias: u32) -> void
        contract { requires n <= a.len && n <= b.len; requires noalias(a, b); effects read(a), write(b); }
        {
          let i: u32 = 0;
          while i < n { b[i] = a[i] * factor + bias; i = i + 1; }
        }
    "#;
    let optimized =
        optimized_simd128_kir(SOURCE, KirWasmFeatures::Simd128, KirOptimizationLevel::O3);
    assert_eq!(optimized.stats.vectorized_loops, 1);
    let module = optimized.artifact.expect("verified u32 vector KIR");
    let options = EmitWasmOptions { opt_level: 3 };
    let wat = emit_wat_kir_module(&module, options).expect("u32 SIMD128 WAT");
    assert!(wat.contains("i32x4.mul"), "{wat}");
    assert!(wat.contains("i32x4.add"), "{wat}");
    let wasm = emit_wasm_kir_module(&module, options).expect("u32 SIMD128 WASM");
    wasmparser::Validator::new_with_features(wasm_features(KirWasmFeatures::Simd128))
        .validate_all(&wasm)
        .expect("u32 SIMD128 module validates");
    run_wasm(
        &wasm,
        r#"
        import fs from "node:fs";
        const { instance } = await WebAssembly.instantiate(fs.readFileSync(process.argv[2]), {});
        const view = new DataView(instance.exports.memory.buffer);
        const input = 260;
        const output = 49156;
        const factor = 0xf1234567;
        const bias = 0xe2345678;
        const values = [0, 1, 0xffffffff, 0x80000000, 0x7fffffff, 0x55555555, 0xaaaaaaaa];
        for (const size of [0, 1, 3, 4, 5, 7, 8, 9, 17, 1025]) {
          view.setUint32(output - 4, 0x13579bdf, true);
          view.setUint32(output + size * 4, 0x13579bdf, true);
          for (let i = 0; i < size; ++i) view.setUint32(input + i * 4, values[i % values.length], true);
          instance.exports.map_u32(input, size, output, size, size, factor, bias);
          for (let i = 0; i < size; ++i) {
            const expected = (Math.imul(values[i % values.length], factor) + bias) >>> 0;
            if (view.getUint32(output + i * 4, true) !== expected) process.exit(1);
          }
          if (view.getUint32(output - 4, true) !== 0x13579bdf ||
              view.getUint32(output + size * 4, true) !== 0x13579bdf) process.exit(2);
        }
    "#,
    );
}

#[test]
fn kir_wasm_simd128_unknown_alias_map_should_emit_runtime_version_predicate() {
    const SOURCE: &str = include_str!("../../examples/wasm/alias_map.ck");
    let optimized =
        optimized_simd128_kir(SOURCE, KirWasmFeatures::Simd128, KirOptimizationLevel::O3);
    assert_eq!(
        optimized.stats.vectorized_loops, 1,
        "{:?}",
        optimized.analysis_fallbacks
    );
    let module = optimized.artifact.expect("verified versioned vector KIR");
    let wat =
        emit_wat_kir_module(&module, EmitWasmOptions { opt_level: 3 }).expect("versioned SIMD WAT");
    assert!(wat.contains("i64.extend_i32_u"), "{wat}");
    assert!(
        wat.contains("v128.load") && wat.contains("v128.store") && wat.contains("i32x4.add"),
        "{wat}"
    );

    let wasm = emit_wasm_kir_module(&module, EmitWasmOptions { opt_level: 3 })
        .expect("versioned SIMD WASM");
    wasmparser::Validator::new_with_features(wasm_features(KirWasmFeatures::Simd128))
        .validate_all(&wasm)
        .expect("versioned SIMD module validates");
    run_wasm(
        &wasm,
        r#"
        import fs from "node:fs";
        import assert from "node:assert/strict";
        const { instance } = await WebAssembly.instantiate(fs.readFileSync(process.argv[2]), {});
        const view = new DataView(instance.exports.memory.buffer);
        const base = 4096;
        const initial = Array.from({length: 112}, (_, i) => (i * 17 + 3) >>> 0);
        for (const [sourceOffset, destinationBase] of [[0, 256], [0, -1], [0, 0], [0, 4], [4, 0]]) {
          for (const size of [0, 1, 2, 3, 4, 5, 7, 8, 9, 17, 31, 33]) {
            const destinationOffset = destinationBase === -1 ? size * 4 : destinationBase;
            initial.forEach((value, i) => view.setUint32(base + i * 4, value, true));
            const reference = initial.slice();
            for (let i = 0; i < size; ++i) {
              reference[destinationOffset / 4 + i] =
                (reference[sourceOffset / 4 + i] + 1) >>> 0;
            }
            instance.exports.alias_map(
              base + sourceOffset, size, base + destinationOffset, size, size);
            assert.deepEqual(
              Array.from({length: initial.length}, (_, i) => view.getUint32(base + i * 4, true)),
              reference,
              `offsets ${sourceOffset}/${destinationOffset}, size ${size}`);
          }
        }
        "#,
    );
}

#[test]
fn kir_wasm_simd128_unknown_alias_f64_vf2_should_match_scalar_for_tails_and_overlap() {
    const SOURCE: &str = r#"
        export fn alias_map_f64(a: slice<f64>, b: slice<f64>, n: u32, delta: f64) -> void {
          let i: u32 = 0;
          while i < n { b[i] = a[i] + delta; i = i + 1; }
        }
    "#;
    let optimized =
        optimized_simd128_kir(SOURCE, KirWasmFeatures::Simd128, KirOptimizationLevel::O3);
    assert_eq!(
        optimized.stats.vectorized_loops, 1,
        "{:?}",
        optimized.analysis_fallbacks
    );
    let module = optimized.artifact.expect("verified f64 VF2 versioned KIR");
    let wat = emit_wat_kir_module(&module, EmitWasmOptions { opt_level: 3 })
        .expect("f64 VF2 versioned WAT");
    assert!(wat.contains("f64x2.add"), "{wat}");
    assert!(wat.contains("i64.extend_i32_u"), "{wat}");
    let wasm = emit_wasm_kir_module(&module, EmitWasmOptions { opt_level: 3 })
        .expect("f64 VF2 versioned WASM");
    run_wasm(
        &wasm,
        r#"
        import fs from "node:fs";
        import assert from "node:assert/strict";
        const { instance } = await WebAssembly.instantiate(fs.readFileSync(process.argv[2]), {});
        const view = new DataView(instance.exports.memory.buffer);
        const base = 8192;
        const seed = [NaN, -0, 0, -Infinity, Infinity, -Number.MAX_VALUE,
                      Number.MAX_VALUE, 1.25, -2.5, Number.MIN_VALUE, 7, 8];
        const initial = Array.from({length: 96}, (_, i) => seed[i % seed.length]);
        const delta = 0.5;
        for (const [sourceOffset, destinationBase] of [[0, 256], [0, -1], [0, 0], [0, 8], [8, 0]]) {
          for (const size of [0, 1, 2, 3, 5, 7, 9]) {
            const destinationOffset = destinationBase === -1 ? size * 8 : destinationBase;
            initial.forEach((value, i) => view.setFloat64(base + i * 8, value, true));
            const reference = initial.slice();
            for (let i = 0; i < size; ++i) {
              reference[destinationOffset / 8 + i] = reference[sourceOffset / 8 + i] + delta;
            }
            instance.exports.alias_map_f64(
              base + sourceOffset, size, base + destinationOffset, size, size, delta);
            for (let i = 0; i < initial.length; ++i) {
              const actual = view.getFloat64(base + i * 8, true);
              const expected = reference[i];
              assert.ok(Object.is(actual, expected),
                `offsets ${sourceOffset}/${destinationOffset}, size ${size}, index ${i}`);
            }
          }
        }
        "#,
    );
}

#[test]
fn kir_wasm_simd128_unknown_alias_i32_should_match_scalar_for_tails_and_overlap() {
    const SOURCE: &str = r#"
        export fn alias_map_i32(a: slice<i32>, b: slice<i32>, n: u32) -> void {
          let i: u32 = 0;
          while i < n { b[i] = a[i] + 1; i = i + 1; }
        }
    "#;
    let optimized =
        optimized_simd128_kir(SOURCE, KirWasmFeatures::Simd128, KirOptimizationLevel::O3);
    assert_eq!(
        optimized.stats.vectorized_loops, 1,
        "{:?}",
        optimized.analysis_fallbacks
    );
    let module = optimized.artifact.expect("verified i32 versioned KIR");
    let wat =
        emit_wat_kir_module(&module, EmitWasmOptions { opt_level: 3 }).expect("i32 versioned WAT");
    assert!(wat.contains("i32x4.add"), "{wat}");
    assert!(wat.contains("i64.extend_i32_u"), "{wat}");
    let wasm = emit_wasm_kir_module(&module, EmitWasmOptions { opt_level: 3 })
        .expect("i32 versioned WASM");
    run_wasm(
        &wasm,
        r#"
        import fs from "node:fs";
        import assert from "node:assert/strict";
        const { instance } = await WebAssembly.instantiate(fs.readFileSync(process.argv[2]), {});
        const view = new DataView(instance.exports.memory.buffer);
        const base = 12288;
        const seed = [-101, -17, -1, 0, 1, 2, 17, 101, -33, 44, -55, 66];
        const initial = Array.from({length: 96}, (_, i) => seed[i % seed.length]);
        for (const [sourceOffset, destinationBase] of [[0, 256], [0, -1], [0, 0], [0, 4], [4, 0]]) {
          for (const size of [0, 1, 3, 4, 5, 7, 9]) {
            const destinationOffset = destinationBase === -1 ? size * 4 : destinationBase;
            initial.forEach((value, i) => view.setInt32(base + i * 4, value, true));
            const reference = initial.slice();
            for (let i = 0; i < size; ++i) {
              reference[destinationOffset / 4 + i] = reference[sourceOffset / 4 + i] + 1;
            }
            instance.exports.alias_map_i32(
              base + sourceOffset, size, base + destinationOffset, size, size);
            for (let i = 0; i < initial.length; ++i) {
              assert.equal(view.getInt32(base + i * 4, true), reference[i],
                `offsets ${sourceOffset}/${destinationOffset}, size ${size}, index ${i}`);
            }
          }
        }
        "#,
    );
}

#[test]
fn kir_wasm_version_predicate_should_widen_wasm32_interval_math_for_all_emitters() {
    const SOURCE: &str = include_str!("../../examples/wasm/alias_map.ck");
    let optimized =
        optimized_simd128_kir(SOURCE, KirWasmFeatures::Simd128, KirOptimizationLevel::O3);
    let module = optimized.artifact.expect("verified versioned vector KIR");
    let predicates = module
        .functions
        .iter()
        .flat_map(|function| &function.blocks)
        .flat_map(|block| &block.instructions)
        .filter_map(|instruction| match &instruction.kind {
            KirInstructionKind::VersionPredicate { predicate } => Some(predicate),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(predicates.len(), 1);
    assert_eq!(predicates[0].address_bits, 32);
    assert_eq!(
        predicates[0]
            .conjuncts
            .iter()
            .filter(|conjunct| matches!(
                conjunct,
                calckernel::KirVersionPredicateConjunct::AddressIntervalsDisjoint { .. }
            ))
            .count(),
        1
    );
    assert!(predicates[0].conjuncts.iter().any(|conjunct| matches!(
        conjunct,
        calckernel::KirVersionPredicateConjunct::AddressIntervalsDisjoint {
            left_element_bytes: 4,
            right_element_bytes: 4,
            ..
        }
    )));

    // The predicate must use widened unsigned arithmetic so a valid interval
    // near 2^32 cannot wrap into a low address. This checks the compiled KIR
    // without allocating a 4 GiB Wasm memory or relying on host pointer width.
    for level in [0, 1, 3] {
        let wat = emit_wat_kir_module(&module, EmitWasmOptions { opt_level: level })
            .unwrap_or_else(|error| panic!("O{level} predicate WAT: {error}"));
        assert!(wat.contains("i64.extend_i32_u"), "O{level}: {wat}");
        assert!(wat.contains("i64.add"), "O{level}: {wat}");
        assert!(
            wat.contains("i64.le_u") || wat.contains("i64.lt_u"),
            "O{level}: {wat}"
        );
        if level == 0 {
            assert!(wat.contains("br_table"), "typed O0 emitter: {wat}");
        }
        let wasm = emit_wasm_kir_module(&module, EmitWasmOptions { opt_level: level })
            .unwrap_or_else(|error| panic!("O{level} predicate WASM: {error}"));
        wasmparser::Validator::new_with_features(wasm_features(KirWasmFeatures::Simd128))
            .validate_all(&wasm)
            .unwrap_or_else(|error| panic!("O{level} predicate WASM validation: {error}"));
    }
}

#[test]
fn kir_wasm_simd128_should_reject_malformed_runtime_predicates() {
    const SOURCE: &str = include_str!("../../examples/wasm/alias_map.ck");
    let optimized =
        optimized_simd128_kir(SOURCE, KirWasmFeatures::Simd128, KirOptimizationLevel::O3);
    let valid = optimized.artifact.expect("verified versioned vector KIR");
    for mutation in 0..3 {
        let mut malformed = valid.clone();
        let predicate = malformed
            .functions
            .iter_mut()
            .flat_map(|function| &mut function.blocks)
            .flat_map(|block| &mut block.instructions)
            .find_map(|instruction| match &mut instruction.kind {
                KirInstructionKind::VersionPredicate { predicate } => Some(predicate),
                _ => None,
            })
            .expect("version predicate");
        match mutation {
            0 => predicate.address_bits = 64,
            1 => {
                let interval = predicate
                    .conjuncts
                    .iter_mut()
                    .find_map(|conjunct| match conjunct {
                        calckernel::KirVersionPredicateConjunct::AddressIntervalsDisjoint {
                            left_element_bytes,
                            ..
                        } => Some(left_element_bytes),
                        _ => None,
                    })
                    .expect("address interval conjunct");
                *interval = 8;
            }
            2 => predicate.conjuncts.clear(),
            _ => unreachable!(),
        }
        let result = std::panic::catch_unwind(|| {
            emit_wat_kir_module(&malformed, EmitWasmOptions { opt_level: 3 })
        });
        assert!(
            result.is_ok(),
            "malformed predicate {mutation} must not panic"
        );
        assert!(
            result.unwrap().is_err(),
            "malformed predicate {mutation} must fail closed"
        );
    }
}

#[test]
fn kir_wasm_simd128_modular_reductions_should_match_baseline_with_initials_tails_and_wrap() {
    const SOURCE: &str = include_str!("../../examples/wasm/reduction.ck");
    const RUNNER: &str = r#"
        import fs from "node:fs";
        import assert from "node:assert/strict";
        const { instance } = await WebAssembly.instantiate(fs.readFileSync(process.argv[2]), {});
        const view = new DataView(instance.exports.memory.buffer);
        const input = 4096;
        // Includes 0..2×VF and each bounded profitability threshold's
        // minimum_trip−1/minimum_trip/minimum_trip+1 boundary (threshold search caps at 1024).
        const sizes = Array.from({length: 1026}, (_, size) => size);
        const u32Values = [0xffffffff, 0x80000001, 0x7fffffff, 3, 5, 0, 17, 0xfffffffd];
        const i32Values = [0x7fffffff, -0x80000000, -1, 3, -5, 0, 17, 0x70000001];
        const cases = [
          {name: "sum_u32", values: u32Values, set: "setUint32", initial: 0xfedcba98,
           step: (total, value) => (total + value) >>> 0, normalize: value => value >>> 0},
          {name: "product_u32", values: u32Values, set: "setUint32", initial: 0x90000003,
           step: (total, value) => Math.imul(total, value) >>> 0, normalize: value => value >>> 0},
          {name: "sum_i32", values: i32Values, set: "setInt32", initial: -0x70000001,
           step: (total, value) => (total + value) | 0, normalize: value => value | 0},
          {name: "product_i32", values: i32Values, set: "setInt32", initial: -0x60000003,
           step: (total, value) => Math.imul(total, value) | 0, normalize: value => value | 0},
        ];
        for (const {name, values, set, initial, step, normalize} of cases) {
          for (const size of sizes) {
            for (let i = 0; i < size; ++i) view[set](input + i * 4, values[i % values.length], true);
            let expected = initial;
            for (let i = 0; i < size; ++i) expected = step(expected, values[i % values.length]);
            assert.equal(normalize(instance.exports[name](input, size, size, initial)), expected,
              `${name} size=${size} initial=${initial}`);
          }
        }
    "#;

    let baseline = optimized_kir(SOURCE, KirOptimizationLevel::O3);
    let baseline_wat = emit_wat_kir_module(&baseline, EmitWasmOptions { opt_level: 3 })
        .expect("baseline reduction WAT");
    assert!(
        !baseline_wat.contains("i32x4"),
        "baseline stays scalar: {baseline_wat}"
    );
    run_wasm(
        &emit_wasm_kir_module(&baseline, EmitWasmOptions { opt_level: 3 })
            .expect("baseline reduction WASM"),
        RUNNER,
    );

    let simd_o0 = optimized_simd128_kir(SOURCE, KirWasmFeatures::Simd128, KirOptimizationLevel::O0);
    assert_eq!(simd_o0.stats.vectorized_loops, 0);
    let simd_o0_module = simd_o0.artifact.expect("SIMD profile O0 reduction KIR");
    let simd_o0_wat = emit_wat_kir_module(&simd_o0_module, EmitWasmOptions { opt_level: 0 })
        .expect("SIMD profile O0 reduction WAT");
    assert!(
        !simd_o0_wat.contains("i32x4"),
        "O0 stays scalar: {simd_o0_wat}"
    );
    run_wasm(
        &emit_wasm_kir_module(&simd_o0_module, EmitWasmOptions { opt_level: 0 })
            .expect("SIMD profile O0 reduction WASM"),
        RUNNER,
    );

    let simd_o3 = optimized_simd128_kir(SOURCE, KirWasmFeatures::Simd128, KirOptimizationLevel::O3);
    assert_eq!(
        simd_o3.stats.vectorized_loops, 4,
        "{:?}",
        simd_o3.analysis_fallbacks
    );
    let simd_o3_module = simd_o3.artifact.expect("SIMD O3 reduction KIR");
    let reduction_count = simd_o3_module
        .functions
        .iter()
        .flat_map(|function| &function.blocks)
        .flat_map(|block| &block.instructions)
        .filter(|instruction| matches!(instruction.kind, KirInstructionKind::VectorReduce { .. }))
        .count();
    assert_eq!(reduction_count, 4);
    let simd_o3_wat = emit_wat_kir_module(&simd_o3_module, EmitWasmOptions { opt_level: 3 })
        .expect("SIMD O3 reduction WAT");
    assert!(
        simd_o3_wat.matches("i32x4.extract_lane").count() >= 16,
        "each of four modular reductions extracts the four lanes: {simd_o3_wat}"
    );
    assert!(
        simd_o3_wat.matches("i32.add").count() >= 6,
        "u32/i32 modular sum reductions need scalar lane folds: {simd_o3_wat}"
    );
    assert!(
        simd_o3_wat.matches("i32.mul").count() >= 6,
        "u32/i32 modular product reductions need scalar lane folds: {simd_o3_wat}"
    );
    run_wasm(
        &emit_wasm_kir_module(&simd_o3_module, EmitWasmOptions { opt_level: 3 })
            .expect("SIMD O3 reduction WASM"),
        RUNNER,
    );
    run_wasm(
        &emit_wasm_kir_module(&simd_o3_module, EmitWasmOptions { opt_level: 0 })
            .expect("SIMD O3 dispatcher reduction WASM"),
        RUNNER,
    );
}

#[test]
fn kir_wasm_simd128_should_reject_unsupported_modular_min_vector_reduction() {
    const SOURCE: &str = include_str!("../../examples/wasm/reduction.ck");
    let optimized =
        optimized_simd128_kir(SOURCE, KirWasmFeatures::Simd128, KirOptimizationLevel::O3);
    let mut malformed = optimized.artifact.expect("verified modular reduction KIR");
    let op = malformed
        .functions
        .iter_mut()
        .flat_map(|function| &mut function.blocks)
        .flat_map(|block| &mut block.instructions)
        .find_map(|instruction| match &mut instruction.kind {
            KirInstructionKind::VectorReduce { op, .. } => Some(op),
            _ => None,
        })
        .expect("vector reduction instruction");
    *op = calckernel::KirVectorReductionOp::ModularMin;

    let result = std::panic::catch_unwind(|| {
        emit_wat_kir_module(&malformed, EmitWasmOptions { opt_level: 3 })
    });
    assert!(result.is_ok(), "unsupported reduction must not panic");
    assert!(
        result.unwrap().is_err(),
        "WebAssembly SIMD128 must fail closed on modular min reduction"
    );
}

fn wasm32_predicate_only_module() -> calckernel::KirModule {
    let left = ValueId::from_index(0);
    let right = ValueId::from_index(1);
    let count = ValueId::from_index(2);
    let result = ValueId::from_index(3);
    let u32_ty = MirType::Primitive(MirPrimitiveTypeName::U32);
    let bool_ty = MirType::Primitive(MirPrimitiveTypeName::Bool);
    calckernel::KirModule {
        config: KirBuildConfig {
            consumer: KirConsumer::WebAssembly,
            overflow_mode: KirOverflowMode::Unchecked,
            bounds_mode: KirBoundsMode::Unchecked,
            sanitizer_mode: KirSanitizerMode::Disabled,
        },
        profile: KirTargetProfile::webassembly_with_features(KirWasmFeatures::Simd128),
        entry: None,
        structs: Vec::new(),
        functions: vec![KirFunction {
            id: FunctionId::from_index(0),
            name: "versioned_disjoint".to_string(),
            exported: true,
            params: vec![
                KirParam {
                    value: left,
                    name: "left".to_string(),
                    type_node: MirType::Slice(Box::new(u32_ty.clone())),
                },
                KirParam {
                    value: right,
                    name: "right".to_string(),
                    type_node: MirType::Slice(Box::new(u32_ty.clone())),
                },
                KirParam {
                    value: count,
                    name: "count".to_string(),
                    type_node: u32_ty,
                },
            ],
            return_type: bool_ty.clone(),
            regions: Vec::new(),
            initial_memory: Vec::new(),
            vector_regions: Vec::new(),
            blocks: vec![KirBlock {
                id: BlockId::from_index(0),
                label: "entry".to_string(),
                params: Vec::new(),
                memory_params: Vec::new(),
                instructions: vec![KirInstruction {
                    id: InstructionId::from_index(0),
                    results: vec![KirResult {
                        value: result,
                        type_node: KirValueType::Scalar(bool_ty),
                    }],
                    kind: KirInstructionKind::VersionPredicate {
                        predicate: KirVersionPredicate {
                            address_bits: 32,
                            conjuncts: vec![
                                KirVersionPredicateConjunct::TripThreshold {
                                    value: count,
                                    minimum: 4,
                                },
                                KirVersionPredicateConjunct::AddressIntervalsDisjoint {
                                    left,
                                    left_count: count,
                                    left_element_bytes: 4,
                                    right,
                                    right_count: count,
                                    right_element_bytes: 4,
                                },
                            ],
                        },
                    },
                    memory: None,
                    effect: None,
                }],
                terminator: KirTerminator::Return {
                    value: Some(result),
                    memory: Vec::new(),
                    effect_order: 0,
                },
            }],
        }],
    }
}

#[test]
fn kir_wasm_predicate_only_hand_kir_should_widen_near_2_to_32_without_vector_values() {
    let module = wasm32_predicate_only_module();
    let version_predicates = module
        .functions
        .iter()
        .flat_map(|function| &function.blocks)
        .flat_map(|block| &block.instructions)
        .filter(|instruction| {
            matches!(
                instruction.kind,
                KirInstructionKind::VersionPredicate { .. }
            )
        })
        .count();
    assert_eq!(version_predicates, 1);
    assert_eq!(
        module
            .functions
            .iter()
            .flat_map(|function| &function.blocks)
            .flat_map(|block| &block.instructions)
            .count(),
        1,
        "predicate-only KIR contains no other instruction, including vector ops"
    );
    assert!(module.functions.iter().all(|function| {
        function.vector_regions.is_empty()
            && function.blocks.iter().all(|block| {
                block
                    .params
                    .iter()
                    .all(|param| param.type_node.as_scalar().is_some())
                    && block.instructions.iter().all(|instruction| {
                        instruction
                            .results
                            .iter()
                            .all(|result| result.type_node.as_scalar().is_some())
                            && !matches!(
                                instruction.kind,
                                KirInstructionKind::VectorSplat { .. }
                                    | KirInstructionKind::VectorLoad { .. }
                                    | KirInstructionKind::VectorStore { .. }
                                    | KirInstructionKind::VectorBinary { .. }
                                    | KirInstructionKind::VectorUnary { .. }
                                    | KirInstructionKind::VectorCompare { .. }
                                    | KirInstructionKind::VectorSelect { .. }
                                    | KirInstructionKind::VectorCast { .. }
                            )
                    })
            })
    }));

    for level in [0, 1, 3] {
        let wat = emit_wat_kir_module(&module, EmitWasmOptions { opt_level: level })
            .unwrap_or_else(|error| panic!("O{level} predicate-only WAT: {error}"));
        assert!(wat.contains("i64.extend_i32_u"), "O{level}: {wat}");
        assert!(wat.contains("i64.add"), "O{level}: {wat}");
        assert!(wat.contains("i64.le_u"), "O{level}: {wat}");
        if level == 0 {
            assert!(
                wat.contains("br_table"),
                "O0 should use typed dispatcher: {wat}"
            );
        }
        let wasm = emit_wasm_kir_module(&module, EmitWasmOptions { opt_level: level })
            .unwrap_or_else(|error| panic!("O{level} predicate-only Wasm: {error}"));
        wasmparser::Validator::new_with_features(wasm_features(KirWasmFeatures::Simd128))
            .validate_all(&wasm)
            .unwrap_or_else(|error| panic!("O{level} Wasm validation: {error}"));
        run_wasm(
            &wasm,
            r#"
            import fs from "node:fs";
            import assert from "node:assert/strict";
            const { instance } = await WebAssembly.instantiate(fs.readFileSync(process.argv[2]), {});
            const check = instance.exports.versioned_disjoint;
            const cases = [
              [0xffffffe0, 0x1000, 4, true],
              [0xfffffff8, 0x1000, 4, false], // exclusive end crosses 2^32
              [0xfffffff8, 0x1000, 1, false], // short trip threshold
              [0x1000, 0x1004, 4, false],
              [0x1000, 0x1000, 4, false],
              [0x1000, 0x2000, 4, true],
            ];
            for (const [left, right, count, expected] of cases) {
              assert.equal(check(left, count, right, count, count), expected ? 1 : 0,
                `left=${left}, right=${right}, count=${count}`);
            }
            "#,
        );
    }
}

#[test]
fn kir_wasm_simd128_integer_to_f64_casts_should_read_exact_eight_byte_inputs() {
    const SOURCE: &str = include_str!("../../examples/wasm/cast_map.ck");
    const RUNNER: &str = r#"
import fs from "node:fs";
import assert from "node:assert/strict";
const { instance } = await WebAssembly.instantiate(fs.readFileSync(process.argv[2]), {});
const memory = instance.exports.memory;
const sizes = [0, 1, 2, 3, 5, 7, 17, 1025];
const signedValues = [-0x80000000, 0x7fffffff, -1, 0, 1, 16777217, -16777217, 0x40000001];
const unsignedValues = [0, 1, 0xffffffff, 0x80000000, 0xfffffffe, 16777217, 0x40000001];
function checkCast(name, values, write, read, output) {
  const view = new DataView(memory.buffer);
  const memoryEnd = view.byteLength;
  const before = 12345.5;
  const after = -67890.25;
  for (const size of sizes) {
    const input = memoryEnd - size * 4;
    view.setFloat64(output - 8, before, true);
    view.setFloat64(output + size * 8, after, true);
    for (let i = 0; i < size; i++) view[write](input + i * 4, values[i % values.length], true);
    instance.exports[name](input, size, output, size, size);
    for (let i = 0; i < size; i++) {
      const expected = Number(values[i % values.length]);
      assert.ok(Object.is(view.getFloat64(output + i * 8, true), expected),
        `${name} size=${size} index=${i}`);
    }
    assert.ok(Object.is(view.getFloat64(output - 8, true), before), `${name} left sentinel`);
    assert.ok(Object.is(view.getFloat64(output + size * 8, true), after), `${name} right sentinel`);
  }
}
checkCast("map_i32_to_f64", signedValues, "setInt32", "getInt32", 4096);
checkCast("map_u32_to_f64", unsignedValues, "setUint32", "getUint32", 16384);
"#;

    for (features, level, emit_level) in [
        (KirWasmFeatures::Baseline, KirOptimizationLevel::O3, 3),
        (KirWasmFeatures::Simd128, KirOptimizationLevel::O0, 0),
    ] {
        let scalar = optimized_simd128_kir(SOURCE, features, level);
        assert_eq!(scalar.stats.vectorized_loops, 0);
        let module = scalar.artifact.as_ref().expect("verified scalar cast KIR");
        let wat = emit_wat_kir_module(
            module,
            EmitWasmOptions {
                opt_level: emit_level,
            },
        )
        .expect("scalar cast WAT");
        assert!(!wat.contains("v128"), "O{emit_level}: {wat}");
        assert!(!wat.contains("f64x2.convert_low"), "{wat}");
        let wasm = emit_wasm_kir_module(
            module,
            EmitWasmOptions {
                opt_level: emit_level,
            },
        )
        .expect("scalar cast Wasm");
        wasmparser::Validator::new_with_features(wasm_features(features))
            .validate_all(&wasm)
            .expect("scalar cast module validates against its profile");
        run_wasm(&wasm, RUNNER);
    }

    let optimized =
        optimized_simd128_kir(SOURCE, KirWasmFeatures::Simd128, KirOptimizationLevel::O3);
    assert_eq!(
        optimized.stats.vectorized_loops, 2,
        "i32/u32 cast maps should each commit: {:?}",
        optimized.analysis_fallbacks
    );
    let module = optimized
        .artifact
        .as_ref()
        .expect("verified cast vector KIR");
    for (name, lane, cast) in [
        (
            "map_i32_to_f64",
            calckernel::KirLaneType::I32,
            calckernel::KirVectorCastOp::I32ToF64,
        ),
        (
            "map_u32_to_f64",
            calckernel::KirLaneType::U32,
            calckernel::KirVectorCastOp::U32ToF64,
        ),
    ] {
        let function = module
            .functions
            .iter()
            .find(|function| function.name == name)
            .expect("cast map function");
        let instructions = function
            .blocks
            .iter()
            .flat_map(|block| &block.instructions)
            .collect::<Vec<_>>();
        let load = instructions
            .iter()
            .find_map(|instruction| match &instruction.kind {
                KirInstructionKind::VectorLoad { access, .. } if access.lane == lane => {
                    Some(access)
                }
                _ => None,
            })
            .expect("two-lane integer vector load");
        assert_eq!(load.lanes, 2);
        assert_eq!(load.byte_footprint, 8);
        let vector_cast = instructions
            .iter()
            .find_map(|instruction| match instruction.kind {
                KirInstructionKind::VectorCast { op, .. } => Some((op, &instruction.results)),
                _ => None,
            })
            .expect("integer-to-f64 vector cast");
        assert_eq!(vector_cast.0, cast);
        assert!(vector_cast.1.iter().any(|result| matches!(
            result.type_node,
            calckernel::KirValueType::FixedVector {
                lane: calckernel::KirLaneType::F64,
                lanes: 2,
            }
        )));
        let store = instructions
            .iter()
            .find_map(|instruction| match &instruction.kind {
                KirInstructionKind::VectorStore { access, .. }
                    if access.lane == calckernel::KirLaneType::F64 =>
                {
                    Some(access)
                }
                _ => None,
            })
            .expect("two-lane f64 vector store");
        assert_eq!(store.lanes, 2);
        assert_eq!(store.byte_footprint, 16);
    }

    let structured =
        emit_wat_kir_module(module, EmitWasmOptions { opt_level: 3 }).expect("structured cast WAT");
    assert!(structured.contains("v128.load64_zero"), "{structured}");
    assert!(
        structured.contains("f64x2.convert_low_i32x4_s"),
        "{structured}"
    );
    assert!(
        structured.contains("f64x2.convert_low_i32x4_u"),
        "{structured}"
    );
    assert!(structured.contains("v128.store"), "{structured}");
    assert!(!structured.contains("br_table"), "{structured}");
    let structured_wasm = emit_wasm_kir_module(module, EmitWasmOptions { opt_level: 3 })
        .expect("structured cast Wasm");
    wasmparser::Validator::new_with_features(wasm_features(KirWasmFeatures::Simd128))
        .validate_all(&structured_wasm)
        .expect("structured cast module validates");
    run_wasm(&structured_wasm, RUNNER);

    let dispatcher = emit_wat_kir_module(module, EmitWasmOptions { opt_level: 0 })
        .expect("typed dispatcher cast WAT");
    assert!(dispatcher.contains("br_table"), "{dispatcher}");
    assert!(dispatcher.contains("v128.load64_zero"), "{dispatcher}");
    assert!(
        dispatcher.contains("f64x2.convert_low_i32x4_s"),
        "{dispatcher}"
    );
    assert!(
        dispatcher.contains("f64x2.convert_low_i32x4_u"),
        "{dispatcher}"
    );
    let dispatcher_wasm = emit_wasm_kir_module(module, EmitWasmOptions { opt_level: 0 })
        .expect("typed dispatcher cast Wasm");
    wasmparser::Validator::new_with_features(wasm_features(KirWasmFeatures::Simd128))
        .validate_all(&dispatcher_wasm)
        .expect("typed dispatcher cast module validates");
    run_wasm(&dispatcher_wasm, RUNNER);

    for (mutation, make_invalid) in [("source footprint", 0_u8), ("lane width", 1_u8)] {
        let mut malformed = module.clone();
        let access = malformed.functions[0]
            .blocks
            .iter_mut()
            .flat_map(|block| &mut block.instructions)
            .find_map(|instruction| match &mut instruction.kind {
                KirInstructionKind::VectorLoad { access, .. } => Some(access),
                _ => None,
            })
            .expect("vector load for malformed access mutation");
        match make_invalid {
            0 => access.byte_footprint = 16,
            1 => access.lanes = 4,
            _ => unreachable!(),
        }
        let emitted = std::panic::catch_unwind(|| {
            emit_wat_kir_module(&malformed, EmitWasmOptions { opt_level: 3 })
        });
        assert!(emitted.is_ok(), "invalid {mutation} must not panic");
        assert!(
            emitted.unwrap().is_err(),
            "invalid {mutation} must fail closed"
        );
    }
}

#[test]
fn kir_wasm_simd128_diamonds_should_compare_select_full_lanes_and_match_scalar() {
    const COMPARISONS: [(&str, calckernel::MirCompareOp); 6] = [
        ("==", calckernel::MirCompareOp::Eq),
        ("!=", calckernel::MirCompareOp::Ne),
        ("<", calckernel::MirCompareOp::Lt),
        ("<=", calckernel::MirCompareOp::Le),
        (">", calckernel::MirCompareOp::Gt),
        (">=", calckernel::MirCompareOp::Ge),
    ];

    for (lane, lanes) in [("f64", 2_u8), ("i32", 4), ("u32", 4)] {
        for (comparison, compare_op) in COMPARISONS {
            let source = pure_diamond_source(lane, comparison);
            let scalar_profiles = [
                (KirWasmFeatures::Baseline, KirOptimizationLevel::O3, 3),
                (KirWasmFeatures::Simd128, KirOptimizationLevel::O0, 0),
            ];
            for (features, level, emit_level) in scalar_profiles {
                let scalar = optimized_simd128_kir(&source, features, level);
                assert_eq!(scalar.stats.vectorized_loops, 0);
                let module = scalar.artifact.as_ref().expect("verified scalar KIR");
                let wat = emit_wat_kir_module(
                    module,
                    EmitWasmOptions {
                        opt_level: emit_level,
                    },
                )
                .expect("scalar comparison WAT");
                assert!(
                    !wat.contains("v128"),
                    "{lane} `{comparison}` O{emit_level}: {wat}"
                );
                assert!(!wat.contains("bitselect"), "{wat}");
                if comparison == "<" {
                    let wasm = emit_wasm_kir_module(
                        module,
                        EmitWasmOptions {
                            opt_level: emit_level,
                        },
                    )
                    .expect("scalar comparison Wasm");
                    run_wasm(&wasm, &compare_select_runner(lane, comparison, lanes));
                }
            }

            let optimized =
                optimized_simd128_kir(&source, KirWasmFeatures::Simd128, KirOptimizationLevel::O3);
            assert_eq!(
                optimized.stats.vectorized_loops, 1,
                "{lane} `{comparison}` did not commit: {:?}",
                optimized.analysis_fallbacks
            );
            let module = optimized.artifact.as_ref().expect("verified vector KIR");
            let compare = module.functions[0]
                .blocks
                .iter()
                .flat_map(|block| &block.instructions)
                .find(|instruction| {
                    matches!(
                        instruction.kind,
                        KirInstructionKind::VectorCompare { op, .. } if op == compare_op
                    )
                })
                .expect("vector comparison instruction");
            assert!(compare.results.iter().any(|result| matches!(
                result.type_node,
                calckernel::KirValueType::Mask { lanes: result_lanes }
                    if result_lanes == u16::from(lanes)
            )));
            assert!(
                module.functions[0]
                    .blocks
                    .iter()
                    .flat_map(|block| &block.instructions)
                    .any(|instruction| matches!(
                        instruction.kind,
                        KirInstructionKind::VectorSelect { .. }
                    ))
            );

            let structured = emit_wat_kir_module(module, EmitWasmOptions { opt_level: 3 })
                .expect("structured SIMD comparison WAT");
            assert!(
                structured.contains(&simd_compare_opcode(lane, comparison)),
                "missing {lane} `{comparison}` SIMD comparison:\n{structured}"
            );
            assert!(structured.contains("v128.bitselect"), "{structured}");
            assert!(
                !structured.contains("br_table"),
                "structured path: {structured}"
            );
            let structured_wasm = emit_wasm_kir_module(module, EmitWasmOptions { opt_level: 3 })
                .expect("structured SIMD comparison Wasm");
            wasmparser::Validator::new_with_features(wasm_features(KirWasmFeatures::Simd128))
                .validate_all(&structured_wasm)
                .expect("structured compare/select Wasm validates");
            run_wasm(
                &structured_wasm,
                &compare_select_runner(lane, comparison, lanes),
            );

            let dispatcher = emit_wat_kir_module(module, EmitWasmOptions { opt_level: 0 })
                .expect("typed dispatcher SIMD comparison WAT");
            assert!(dispatcher.contains("br_table"), "{dispatcher}");
            assert!(
                dispatcher.contains(&simd_compare_opcode(lane, comparison)),
                "{dispatcher}"
            );
            assert!(dispatcher.contains("v128.bitselect"), "{dispatcher}");
            let dispatcher_wasm = emit_wasm_kir_module(module, EmitWasmOptions { opt_level: 0 })
                .expect("typed dispatcher SIMD comparison Wasm");
            wasmparser::Validator::new_with_features(wasm_features(KirWasmFeatures::Simd128))
                .validate_all(&dispatcher_wasm)
                .expect("typed dispatcher compare/select Wasm validates");
            run_wasm(
                &dispatcher_wasm,
                &compare_select_runner(lane, comparison, lanes),
            );

            if lane == "f64" && comparison == "==" {
                let mut malformed = module.clone();
                let result = malformed.functions[0]
                    .blocks
                    .iter_mut()
                    .flat_map(|block| &mut block.instructions)
                    .find_map(|instruction| match &mut instruction.kind {
                        KirInstructionKind::VectorCompare { .. } => {
                            Some(&mut instruction.results[0].type_node)
                        }
                        _ => None,
                    })
                    .expect("vector mask result");
                *result = calckernel::KirValueType::Mask { lanes: 4 };
                let emitted = std::panic::catch_unwind(|| {
                    emit_wat_kir_module(&malformed, EmitWasmOptions { opt_level: 3 })
                });
                assert!(emitted.is_ok(), "invalid mask shape must not panic");
                assert!(
                    emitted.unwrap().is_err(),
                    "invalid mask shape must fail closed"
                );

                let mut malformed_select = module.clone();
                let vector_value = malformed_select.functions[0]
                    .blocks
                    .iter()
                    .flat_map(|block| &block.instructions)
                    .flat_map(|instruction| &instruction.results)
                    .find_map(|result| {
                        matches!(
                            result.type_node,
                            calckernel::KirValueType::FixedVector { .. }
                        )
                        .then_some(result.value)
                    })
                    .expect("vector value for malformed select mutation");
                let select_mask = malformed_select.functions[0]
                    .blocks
                    .iter_mut()
                    .flat_map(|block| &mut block.instructions)
                    .find_map(|instruction| match &mut instruction.kind {
                        KirInstructionKind::VectorSelect { mask, .. } => Some(mask),
                        _ => None,
                    })
                    .expect("vector select mask");
                *select_mask = vector_value;
                let emitted = std::panic::catch_unwind(|| {
                    emit_wat_kir_module(&malformed_select, EmitWasmOptions { opt_level: 3 })
                });
                assert!(emitted.is_ok(), "invalid select mask must not panic");
                assert!(
                    emitted.unwrap().is_err(),
                    "vector select with a data vector as its mask must fail closed"
                );
            }
        }
    }
}

#[test]
fn kir_wasm_o3_should_stackify_single_use_and_tee_multi_use_scalars() {
    const SOURCE: &str = r#"
        export fn i64_chain(a: i64, b: i64, c: i64) -> i64 {
          let first: i64 = a + b;
          let second: i64 = first * c;
          return second;
        }
        export fn i64_multi(a: i64, b: i64) -> i64 {
          let shared: i64 = a + b;
          return shared * shared;
        }
        export fn f64_chain(a: f64, b: f64, c: f64) -> f64 {
          let first: f64 = a + b;
          let second: f64 = first * c;
          return second;
        }
        export fn i64_disjoint(a: i64, b: i64, c: i64, d: i64) -> i64 {
          let first: i64 = a + b;
          let first_square: i64 = first * first;
          let second: i64 = c + d;
          let second_square: i64 = second * second;
          return first_square + second_square;
        }
    "#;
    let kir = optimized_kir(SOURCE, KirOptimizationLevel::O0);
    assert_eq!(calckernel::validate_kir_module(&kir).errors, []);

    let emit = |opt_level| {
        emit_wat_kir_module(&kir, EmitWasmOptions { opt_level }).expect("verified KIR WAT")
    };
    let o0 = emit(0);
    let o2 = emit(2);
    let o3 = emit(3);
    wat::parse_str(&o3).expect("O3 WAT syntax");

    assert!(
        !o0.contains("local.tee"),
        "O0 physical placement changed:\n{o0}"
    );
    assert!(
        !o2.contains("local.tee"),
        "O2 physical placement changed:\n{o2}"
    );
    assert!(
        o3.contains("local.tee"),
        "O3 should keep a multi-use scalar on the stack while saving it:\n{o3}"
    );
    assert!(
        o3.matches("(local $").count() < o2.matches("(local $").count(),
        "O3 should reduce physical locals for single-use scalar chains; O2={} O3={}\n{o3}",
        o2.matches("(local $").count(),
        o3.matches("(local $").count(),
    );
    let disjoint = o3
        .split("(func $i64_disjoint")
        .nth(1)
        .expect("coalescing function in O3 WAT")
        .split("\n  )")
        .next()
        .expect("coalescing function body");
    let tee_targets = disjoint
        .lines()
        .filter_map(|line| line.trim().strip_prefix("local.tee $").map(str::to_owned))
        .collect::<Vec<_>>();
    assert_eq!(
        tee_targets.len(),
        2,
        "both disjoint multi-use values should be saved with local.tee:\n{disjoint}"
    );
    assert_eq!(
        tee_targets[0], tee_targets[1],
        "non-overlapping live ranges should reuse one physical local:\n{disjoint}"
    );

    run_wasm(
        &emit_wasm_kir_module(&kir, EmitWasmOptions { opt_level: 3 }).expect("O3 WASM"),
        r#"
import { readFileSync } from "node:fs";
const { instance } = await WebAssembly.instantiate(readFileSync(process.argv[2]));
if (instance.exports.i64_chain(4n, 7n, 3n) !== 33n) process.exit(1);
if (instance.exports.i64_multi(4n, 7n) !== 121n) process.exit(2);
if (instance.exports.f64_chain(1.5, 2.25, 4) !== 15) process.exit(3);
if (instance.exports.i64_disjoint(2n, 5n, 3n, 4n) !== 98n) process.exit(4);
"#,
    );
}

#[test]
fn kir_wasm_backend_should_keep_dispatcher_for_validated_irreducible_cfg() {
    let mut kir = optimized_kir(
        "export fn irreducible(flag: bool) -> void { if flag { return; } return; }",
        KirOptimizationLevel::O0,
    );
    let function = kir
        .functions
        .iter_mut()
        .find(|function| function.name == "irreducible")
        .expect("irreducible function");
    let (then_target, else_target) = function
        .blocks
        .iter()
        .find_map(|block| match &block.terminator {
            KirTerminator::Branch {
                then_edge,
                else_edge,
                ..
            } if then_edge.target != else_edge.target => Some((then_edge.target, else_edge.target)),
            _ => None,
        })
        .expect("conditional entry into two distinct blocks");
    assert_ne!(then_target, else_target);
    for (source, target) in [(then_target, else_target), (else_target, then_target)] {
        let source_block = function
            .blocks
            .iter()
            .find(|block| block.id == source)
            .expect("branch source");
        let args = source_block
            .params
            .iter()
            .map(|param| param.value)
            .collect();
        let memory_args = source_block
            .memory_params
            .iter()
            .map(|param| param.version)
            .collect();
        let block = function
            .blocks
            .iter_mut()
            .find(|block| block.id == source)
            .expect("branch source");
        block.terminator = KirTerminator::Jump {
            edge: KirEdge {
                target,
                args,
                memory_args,
            },
        };
    }
    assert_eq!(
        calckernel::validate_kir_module(&kir).errors,
        [],
        "two-entry strongly connected component should remain valid KIR"
    );

    let wat = emit_wat_kir_module(&kir, EmitWasmOptions { opt_level: 3 })
        .expect("irreducible KIR should fall back to dispatcher");
    assert!(wat.contains("loop $ik_dispatch"), "{wat}");
    assert!(wat.contains("br_table"), "{wat}");
}

#[test]
fn kir_wasm_backend_should_structure_same_target_branch_with_slice_edge_arguments() {
    const SOURCE: &str = r#"
        fn choose(items: slice<i32>, other: slice<i32>, use_other: bool) -> slice<i32> {
          let selected: slice<i32> = items;
          if use_other { selected = other; }
          return selected;
        }
        export fn selected(items: slice<i32>, other: slice<i32>, use_other: bool) -> i32 {
          let chosen: slice<i32> = choose(items, other, use_other);
          return chosen[0];
        }
    "#;
    let mut kir = optimized_kir(SOURCE, KirOptimizationLevel::O0);
    let function = kir
        .functions
        .iter_mut()
        .find(|function| function.name == "choose")
        .expect("choose function");
    let branch_block = function
        .blocks
        .iter()
        .find(|block| matches!(block.terminator, KirTerminator::Branch { .. }))
        .expect("branch block");
    let KirTerminator::Branch {
        condition,
        then_edge,
        else_edge,
    } = &branch_block.terminator
    else {
        unreachable!();
    };
    let condition = *condition;
    let first_target = then_edge.target;
    let second_target = else_edge.target;
    let then_memory_args = then_edge.memory_args.clone();
    let else_memory_args = else_edge.memory_args.clone();
    let join_target = function
        .blocks
        .iter()
        .find(|block| block.id == first_target)
        .and_then(|first| match &first.terminator {
            KirTerminator::Jump { edge } => Some(edge.target),
            _ => None,
        })
        .filter(|target| *target == second_target)
        .or_else(|| {
            function
                .blocks
                .iter()
                .find(|block| block.id == second_target)
                .and_then(|second| match &second.terminator {
                    KirTerminator::Jump { edge } if edge.target == first_target => {
                        Some(first_target)
                    }
                    _ => None,
                })
        })
        .expect("branch arms should flow to one join block");
    let join = function
        .blocks
        .iter()
        .find(|block| block.id == join_target)
        .expect("join block");
    let items = function
        .params
        .iter()
        .find(|param| param.name == "items")
        .expect("items parameter")
        .value;
    let other = function
        .params
        .iter()
        .find(|param| param.name == "other")
        .expect("other parameter")
        .value;
    assert_ne!(items, other);
    let flag = function
        .params
        .iter()
        .find(|param| param.name == "use_other")
        .expect("condition parameter")
        .value;
    let join_args = |select_other: bool| {
        join.params
            .iter()
            .map(|param| match param.slot.as_str() {
                "items" => items,
                "other" => other,
                "use_other" => flag,
                "selected" if select_other => other,
                "selected" => items,
                slot => panic!("unexpected join parameter {slot}"),
            })
            .collect::<Vec<_>>()
    };
    assert!(join.params.iter().any(|param| {
        matches!(
            &param.type_node,
            calckernel::KirValueType::Scalar(calckernel::MirType::Slice(_))
        )
    }));
    let then_args = join_args(true);
    let else_args = join_args(false);
    assert_ne!(
        then_args, else_args,
        "the slice edge pair differs by branch arm"
    );

    let branch = function
        .blocks
        .iter_mut()
        .find(|block| {
            matches!(
                &block.terminator,
                KirTerminator::Branch { condition: branch_condition, .. }
                    if *branch_condition == condition
            )
        })
        .expect("original branch block");
    branch.terminator = KirTerminator::Branch {
        condition,
        then_edge: KirEdge {
            target: join_target,
            args: then_args,
            memory_args: then_memory_args,
        },
        else_edge: KirEdge {
            target: join_target,
            args: else_args,
            memory_args: else_memory_args,
        },
    };
    assert_eq!(calckernel::validate_kir_module(&kir).errors, []);

    let runner = r#"
        import fs from "node:fs";
        const { instance } = await WebAssembly.instantiate(fs.readFileSync(process.argv[2]), {});
        const memory = new Int32Array(instance.exports.memory.buffer);
        memory[128 / 4] = 10; memory[132 / 4] = 11;
        memory[192 / 4] = 30; memory[196 / 4] = 31;
        if (instance.exports.selected(128, 2, 192, 2, 0) !== 10) process.exit(1);
        if (instance.exports.selected(128, 2, 192, 2, 1) !== 30) process.exit(2);
        "#;
    for level in [0, 2, 3] {
        let options = EmitWasmOptions { opt_level: level };
        let wasm = emit_wasm_kir_module(&kir, options).expect("same-target slice WASM");
        run_wasm(&wasm, runner);
    }
    let wat =
        emit_wat_kir_module(&kir, EmitWasmOptions { opt_level: 3 }).expect("same-target slice WAT");
    assert!(
        !wat.contains("ik_dispatch") && !wat.contains("br_table"),
        "same-target edge arguments should be structured:\n{wat}"
    );
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
    assert!(
        !wat.contains("ik_dispatch") && !wat.contains("br_table"),
        "a proof-loop branch whose arms pass different values to one merge should be structured:\n{wat}"
    );
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
        if (instance.exports.kernel(256, 4, 50n) !== 50n) process.exit(2);
        "#,
    );
}

#[test]
fn scalar_unroll_wasm_should_match_o0_for_full_and_partial_exact_remainder() {
    let source = r#"
        export fn full() -> u32 {
          let i: u32 = 0; let total: u32 = 0;
          while i < 8 { total = total + i; i = i + 1; }
          return total;
        }
        export fn partial() -> u32 {
          let i: u32 = 0; let total: u32 = 0;
          while i < 11 { total = total + i; i = i + 1; }
          return total;
        }
    "#;
    let runner = r#"
        import fs from "node:fs";
        const bytes = fs.readFileSync(process.argv[2]);
        const { instance } = await WebAssembly.instantiate(bytes, {});
        if (instance.exports.full() !== 28) process.exit(1);
        if (instance.exports.partial() !== 55) process.exit(2);
    "#;
    for (level, numeric) in [(KirOptimizationLevel::O0, 0), (KirOptimizationLevel::O3, 3)] {
        let kir = optimized_kir(source, level);
        assert!(
            kir.functions
                .iter()
                .all(|function| function.vector_regions.is_empty())
        );
        let wasm = emit_wasm_kir_module(&kir, EmitWasmOptions { opt_level: numeric })
            .expect("scalar unroll WASM");
        run_wasm(&wasm, runner);
    }
}

#[test]
fn loop_simd_portable_wasm_should_remain_scalar_and_match_o0() {
    let source = r#"
        export unsafe fn map(a: slice<u32>, b: slice<u32>, n: u32) -> void
        contract { requires noalias(a, b); effects read(a), write(b); }
        {
          let i: u32 = 0;
          while i < n { b[i] = a[i] + 7; i = i + 1; }
        }
    "#;
    let runner = r#"
        import fs from "node:fs";
        const bytes = fs.readFileSync(process.argv[2]);
        const { instance } = await WebAssembly.instantiate(bytes, {});
        const memory = new Uint32Array(instance.exports.memory.buffer);
        const input = 256 / 4;
        const output = 512 / 4;
        for (let i = 0; i < 16; ++i) memory[input + i] = i * 3;
        instance.exports.map(256, 16, 512, 16, 16);
        for (let i = 0; i < 16; ++i) {
          if (memory[output + i] !== memory[input + i] + 7) process.exit(1);
        }
    "#;
    for (level, numeric) in [(KirOptimizationLevel::O0, 0), (KirOptimizationLevel::O3, 3)] {
        let kir = optimized_kir(source, level);
        assert!(
            kir.functions
                .iter()
                .all(|function| function.vector_regions.is_empty())
        );
        let wasm = emit_wasm_kir_module(&kir, EmitWasmOptions { opt_level: numeric })
            .expect("scalar loop SIMD WASM");
        run_wasm(&wasm, runner);
    }
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
