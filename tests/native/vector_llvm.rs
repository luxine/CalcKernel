use std::{fs, process::Command};

use calckernel::{
    BlockId, EmitLlvmOptions, FunctionId, InstructionId, KirArithmeticSemantics, KirBlock,
    KirBoundsMode, KirBuildConfig, KirConsumer, KirEffectKind, KirFunction, KirInitialMemory,
    KirInstruction, KirInstructionKind, KirLaneType, KirMemoryAccess, KirMemoryRegion,
    KirMemoryRegionOrigin, KirModule, KirOptimizationLevel, KirOrderedEffect, KirOverflowMode,
    KirParam, KirResult, KirSanitizerMode, KirTerminator, KirValueType, KirVectorBinaryOp,
    KirVectorCastOp, KirVectorMemoryAccess, KirVectorReductionOp, KirVectorRegion,
    KirVectorUnaryOp, MemoryRegionId, MemoryVersionId, MirCompareOp, MirPrimitiveTypeName, MirType,
    NativeContext, NativeCpu, NativeHeaderMode, NativeOptimizationLevel, NativeTarget, SourceFile,
    ValueId, VectorRegionId, build_kir_module_with_profile, check, emit_native_header,
    import_contract_facts, lower_native_kir_module, lower_to_mir, print_kir_module,
    run_kir_pass_pipeline,
};

const LOOP_SIMD_SOURCE: &str = r#"
export unsafe fn map_u32(a: slice<u32>, b: slice<u32>, n: u32) -> void
contract { requires noalias(a, b); effects read(a), write(b); }
{
  let i: u32 = 0;
  while i < n { b[i] = a[i] + 7; i = i + 1; }
}
"#;

const VERSIONED_LOOP_SIMD_SOURCE: &str = r#"
export fn map_unknown(a: slice<u32>, b: slice<u32>, n: u32) -> void {
  let i: u32 = 0;
  while i < n { b[i] = a[i] + 1; i = i + 1; }
}
"#;

const STRICT_F64_LOOP_SIMD_SOURCE: &str = r#"
export unsafe fn map_f64(a: slice<f64>, b: slice<f64>, n: u32, factor: f64) -> void
contract { requires noalias(a, b); effects read(a), write(b); }
{
  let i: u32 = 0;
  while i < n { b[i] = a[i] * factor; i = i + 1; }
}
"#;

const STRICT_F64_UNARY_DIVIDE_LOOP_SIMD_SOURCE: &str = r#"
export unsafe fn map_f64_div(
  a: slice<f64>, b: slice<f64>, n: u32, divisor: f64
) -> void
contract { requires noalias(a, b); effects read(a), write(b); }
{
  let i: u32 = 0;
  while i < n { b[i] = -a[i] / divisor; i = i + 1; }
}
"#;

const REDUCTION_LOOP_SIMD_SOURCE: &str = r#"
export fn sum_u32(a: slice<u32>, n: u32) -> u32 {
  let i: u32 = 0;
  let total: u32 = 0;
  while i < n { total = total + a[i]; i = i + 1; }
  return total;
}
export fn product_u32(a: slice<u32>, n: u32) -> u32 {
  let i: u32 = 0;
  let total: u32 = 1;
  while i < n { total = total * a[i]; i = i + 1; }
  return total;
}
"#;

const CAST_AND_DIAMOND_LOOP_SIMD_SOURCE: &str = r#"
export unsafe fn map_cast(a: slice<u32>, b: slice<f64>, n: u32) -> void
contract { requires noalias(a, b); effects read(a), write(b); }
{
  let i: u32 = 0;
  while i < n { b[i] = u32_to_f64(a[i]); i = i + 1; }
}

export unsafe fn map_diamond(
  a: slice<u32>, b: slice<u32>, n: u32, pivot: u32
) -> void
contract { requires noalias(a, b); effects read(a), write(b); }
{
  let i: u32 = 0;
  while i < n {
    let x: u32 = a[i];
    let selected: u32 = 0;
    if x < pivot { selected = x + 1; } else { selected = x - 1; }
    b[i] = selected;
    i = i + 1;
  }
}
"#;

fn scalar(name: MirPrimitiveTypeName) -> KirValueType {
    KirValueType::Scalar(MirType::Primitive(name))
}

fn vector(lane: KirLaneType) -> KirValueType {
    KirValueType::FixedVector { lane, lanes: 4 }
}

fn result(value: u32, type_node: KirValueType) -> Vec<KirResult> {
    vec![KirResult {
        value: ValueId::from_index(value),
        type_node,
    }]
}

fn instruction(
    id: u32,
    results: Vec<KirResult>,
    kind: KirInstructionKind,
    memory: Option<KirMemoryAccess>,
    effect: Option<KirOrderedEffect>,
) -> KirInstruction {
    KirInstruction {
        id: InstructionId::from_index(id),
        results,
        kind,
        memory,
        effect,
    }
}

fn access() -> KirVectorMemoryAccess {
    KirVectorMemoryAccess {
        slice: ValueId::from_index(1),
        start: ValueId::from_index(2),
        end: ValueId::from_index(3),
        lane: KirLaneType::I32,
        lanes: 4,
        byte_footprint: 16,
        known_alignment: 4,
        required_alignment: 4,
    }
}

pub(super) fn vector_module(target: &NativeTarget) -> KirModule {
    let vector_region = VectorRegionId::from_index(0);
    let memory_region = MemoryRegionId::from_index(0);
    let memory0 = MemoryVersionId::from_index(0);
    let memory1 = MemoryVersionId::from_index(1);
    let block = BlockId::from_index(0);
    let instructions = vec![
        instruction(
            0,
            result(10, vector(KirLaneType::I32)),
            KirInstructionKind::VectorSplat {
                scalar: ValueId::from_index(0),
                region: vector_region,
            },
            None,
            None,
        ),
        instruction(
            1,
            result(11, vector(KirLaneType::I32)),
            KirInstructionKind::VectorLoad {
                access: access(),
                region: vector_region,
            },
            Some(KirMemoryAccess {
                region: memory_region,
                input: memory0,
                output: None,
            }),
            Some(KirOrderedEffect {
                order: 0,
                kind: KirEffectKind::ReadMemory,
            }),
        ),
        instruction(
            2,
            result(12, vector(KirLaneType::I32)),
            KirInstructionKind::VectorBinary {
                op: KirVectorBinaryOp::Add,
                left: ValueId::from_index(10),
                right: ValueId::from_index(11),
                semantics: KirArithmeticSemantics::Modular,
                no_failure_proof: None,
                region: vector_region,
            },
            None,
            None,
        ),
        instruction(
            3,
            result(13, KirValueType::Mask { lanes: 4 }),
            KirInstructionKind::VectorCompare {
                op: MirCompareOp::Lt,
                left: ValueId::from_index(10),
                right: ValueId::from_index(11),
                region: vector_region,
            },
            None,
            None,
        ),
        instruction(
            4,
            result(14, KirValueType::Mask { lanes: 4 }),
            KirInstructionKind::VectorUnary {
                op: KirVectorUnaryOp::MaskNot,
                operand: ValueId::from_index(13),
                semantics: KirArithmeticSemantics::Modular,
                no_failure_proof: None,
                region: vector_region,
            },
            None,
            None,
        ),
        instruction(
            5,
            result(15, vector(KirLaneType::I32)),
            KirInstructionKind::VectorSelect {
                mask: ValueId::from_index(14),
                when_true: ValueId::from_index(12),
                when_false: ValueId::from_index(11),
                region: vector_region,
            },
            None,
            None,
        ),
        instruction(
            6,
            result(16, vector(KirLaneType::I32)),
            KirInstructionKind::VectorUnary {
                op: KirVectorUnaryOp::Negate,
                operand: ValueId::from_index(15),
                semantics: KirArithmeticSemantics::Modular,
                no_failure_proof: None,
                region: vector_region,
            },
            None,
            None,
        ),
        instruction(
            7,
            result(17, vector(KirLaneType::F64)),
            KirInstructionKind::VectorCast {
                op: KirVectorCastOp::I32ToF64,
                value: ValueId::from_index(16),
                region: vector_region,
            },
            None,
            None,
        ),
        instruction(
            8,
            result(18, vector(KirLaneType::F64)),
            KirInstructionKind::VectorBinary {
                op: KirVectorBinaryOp::Add,
                left: ValueId::from_index(17),
                right: ValueId::from_index(17),
                semantics: KirArithmeticSemantics::StrictFloat,
                no_failure_proof: None,
                region: vector_region,
            },
            None,
            None,
        ),
        instruction(
            9,
            result(19, vector(KirLaneType::I32)),
            KirInstructionKind::VectorInsert {
                vector: ValueId::from_index(12),
                scalar: ValueId::from_index(4),
                lane_index: 2,
                region: vector_region,
            },
            None,
            None,
        ),
        instruction(
            10,
            result(20, scalar(MirPrimitiveTypeName::I32)),
            KirInstructionKind::VectorExtract {
                vector: ValueId::from_index(19),
                lane_index: 2,
                region: vector_region,
            },
            None,
            None,
        ),
        instruction(
            11,
            result(21, scalar(MirPrimitiveTypeName::I32)),
            KirInstructionKind::VectorReduce {
                op: KirVectorReductionOp::ModularAdd,
                vector: ValueId::from_index(12),
                semantics: KirArithmeticSemantics::Modular,
                region: vector_region,
            },
            None,
            None,
        ),
        instruction(
            12,
            Vec::new(),
            KirInstructionKind::VectorStore {
                access: access(),
                value: ValueId::from_index(12),
                region: vector_region,
            },
            Some(KirMemoryAccess {
                region: memory_region,
                input: memory0,
                output: Some(memory1),
            }),
            Some(KirOrderedEffect {
                order: 1,
                kind: KirEffectKind::WriteMemory,
            }),
        ),
    ];
    KirModule {
        config: KirBuildConfig {
            consumer: KirConsumer::NativeLibrary,
            overflow_mode: KirOverflowMode::Unchecked,
            bounds_mode: KirBoundsMode::Unchecked,
            sanitizer_mode: KirSanitizerMode::Disabled,
        },
        profile: target
            .kir_profile(KirConsumer::NativeLibrary)
            .expect("target profile"),
        entry: None,
        structs: Vec::new(),
        functions: vec![KirFunction {
            id: FunctionId::from_index(0),
            name: "vector_kernel".to_string(),
            exported: true,
            params: vec![
                KirParam {
                    value: ValueId::from_index(0),
                    name: "scalar".to_string(),
                    type_node: MirType::Primitive(MirPrimitiveTypeName::I32),
                },
                KirParam {
                    value: ValueId::from_index(1),
                    name: "items".to_string(),
                    type_node: MirType::Slice(Box::new(MirType::Primitive(
                        MirPrimitiveTypeName::I32,
                    ))),
                },
                KirParam {
                    value: ValueId::from_index(2),
                    name: "start".to_string(),
                    type_node: MirType::Primitive(MirPrimitiveTypeName::U32),
                },
                KirParam {
                    value: ValueId::from_index(3),
                    name: "end".to_string(),
                    type_node: MirType::Primitive(MirPrimitiveTypeName::U32),
                },
                KirParam {
                    value: ValueId::from_index(4),
                    name: "replacement".to_string(),
                    type_node: MirType::Primitive(MirPrimitiveTypeName::I32),
                },
            ],
            return_type: MirType::Primitive(MirPrimitiveTypeName::I32),
            regions: vec![KirMemoryRegion {
                id: memory_region,
                origin: KirMemoryRegionOrigin::Parameter(ValueId::from_index(1)),
                parent: None,
                partition: memory_region,
                byte_interval: None,
            }],
            initial_memory: vec![KirInitialMemory {
                region: memory_region,
                version: memory0,
            }],
            vector_regions: vec![KirVectorRegion {
                id: vector_region,
                blocks: vec![block],
            }],
            blocks: vec![KirBlock {
                id: block,
                label: "entry".to_string(),
                params: Vec::new(),
                memory_params: Vec::new(),
                instructions,
                terminator: KirTerminator::Return {
                    value: Some(ValueId::from_index(21)),
                    memory: vec![(memory_region, memory1)],
                    effect_order: 2,
                },
            }],
        }],
    }
}

#[test]
fn vector_llvm_should_lower_every_closed_family_to_fixed_vector_ir() {
    let context = NativeContext::new().expect("context");
    let target = NativeTarget::host_with_cpu(NativeCpu::Baseline).expect("target");
    let result = run_kir_pass_pipeline(vector_module(&target), KirOptimizationLevel::O0, None);
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    let text = lower_native_kir_module(&context, &target, &result, &EmitLlvmOptions::default())
        .expect("vector lowering")
        .verify()
        .expect("vector LLVM verification")
        .to_ir_string()
        .expect("vector LLVM IR");

    for spelling in [
        "<4 x i32>",
        "load <4 x i32>",
        "store <4 x i32>",
        "add <4 x i32>",
        "icmp slt <4 x i32>",
        "select <4 x i1>",
        "sitofp <4 x i32>",
        "fadd <4 x double>",
        "insertelement <4 x i32>",
        "extractelement <4 x i32>",
        "llvm.vector.reduce.add.v4i32",
        "align 4",
    ] {
        assert!(text.contains(spelling), "missing {spelling}:\n{text}");
    }
    let fadd = text
        .lines()
        .find(|line| line.contains("fadd <4 x double>"))
        .expect("strict vector fadd");
    assert!(
        !fadd.contains(" fast ") && !fadd.contains("contract"),
        "{fadd}"
    );
}

fn has_aarch64_packed_i32_add(disassembly: &str) -> bool {
    disassembly.lines().any(|line| {
        let instruction = line.split("//").next().unwrap_or(line).to_ascii_lowercase();
        let tokens: Vec<_> = instruction.split_whitespace().collect();
        let Some(mnemonic_index) = tokens
            .iter()
            .position(|token| matches!(*token, "add" | "add.2s" | "add.4s"))
        else {
            return false;
        };
        let mnemonic_has_lane_suffix = match tokens[mnemonic_index] {
            "add" => false,
            "add.2s" | "add.4s" => true,
            _ => return false,
        };
        let operands_text = tokens[mnemonic_index + 1..].join(" ");
        let operands: Vec<_> = operands_text.split(',').map(str::trim).collect();
        if operands.len() != 3 {
            return false;
        }
        if mnemonic_has_lane_suffix {
            return operands
                .iter()
                .all(|operand| aarch64_vector_register(operand));
        }
        let Some(lane_suffix) = aarch64_packed_i32_register_lane(operands[0]) else {
            return false;
        };
        operands[1..]
            .iter()
            .all(|operand| aarch64_packed_i32_register_lane(operand) == Some(lane_suffix))
    })
}

fn aarch64_vector_register(operand: &str) -> bool {
    let Some(number) = operand.strip_prefix('v') else {
        return false;
    };
    aarch64_vector_register_number(number).is_some()
}

fn aarch64_vector_register_number(number: &str) -> Option<u8> {
    if number.is_empty()
        || !number.bytes().all(|byte| byte.is_ascii_digit())
        || (number.len() > 1 && number.starts_with('0'))
    {
        return None;
    }
    let number = number.parse::<u8>().ok()?;
    (number < 32).then_some(number)
}

fn aarch64_packed_i32_register_lane(operand: &str) -> Option<&str> {
    let register = operand.strip_prefix('v')?;
    let (number, lane_suffix) = register.split_once('.')?;
    if !matches!(lane_suffix, "2s" | "4s") {
        return None;
    }
    aarch64_vector_register_number(number)?;
    Some(lane_suffix)
}

#[test]
fn aarch64_object_contract_requires_packed_integer_add() {
    for disassembly in [
        "0: 0e210c20 add v0.2s, v1.2s, v2.2s",
        "0: 4ea28420 add v0.4s, v1.4s, v2.4s",
        "28: 0ea08421 add.2s v1, v1, v0",
        "94: 4ea08421 add.4s v1, v1, v0",
        "98: 4ea08442 add.4s v2, v2, v0",
    ] {
        assert!(
            has_aarch64_packed_i32_add(disassembly),
            "expected packed integer ADD in {disassembly}"
        );
    }

    for disassembly in [
        "0: 8b020020 add x0, x1, x2",
        "0: 11000908 add w0, w1, w2",
        "0: 0ea08421 add.2s w0, w1, w2",
        "0: 1e622820 fadd d0, d1, d2",
        "0: 4e228420 fadd v0.2s, v1.2s, v2.2s",
        "0: 0ea08421 fadd.2s v0, v1, v2",
        "0: 0ea08421 addp.2s v0, v1, v2",
        "0: 3dc00020 ldr q0, [x1]",
        "0: 3d800020 str q0, [x1]",
        "0: 3dc00020 ldr.4s v0, [x1]",
        "0: 3d800020 str.2s v0, [x1]",
        "0: 0e210c20 add v0.2s, d1, v2.2s",
        "0: 0e210c20 add v0.2s, v1.4s, v2.2s",
        "0: 4ea08421 add.4s v0, d1, v2",
        "0: 0ea08421 add.2s v0, v1.4s, v2",
        "0: 0ea08421 add.2s v0.4s, v1.4s, v2.4s",
        "0: 0ea08421 add.2s v0.2s, v1.2s, v2.2s",
        "0: 0ea08421 add.2s v0, v1.2s, v2",
        "0: 0ea08421 add.2d v0, v1, v2",
        "0: 0ea08421 add.8s v0, v1, v2",
        "0: 0e210c20 add v0.8h, v1.8h, v2.8h",
        "0: 0ea08421 add.2s v32, v1, v2",
        "0: 0ea08421 add.2s v01, v1, v2",
        "0: 0e210c20 add v01.2s, v1.2s, v2.2s",
        "0: 0ea08421 add.2s v0, v1",
        "0: 0ea08421 add.2s v0, v1, v2, v3",
        "0: 0ea08421 add.2s v0, v1, v2, #1",
        "0: 0ea08421 add.2s v+1, v1, v2",
        "0: 8b020020 add x0, x1, x2 // add.2s v0, v1, v2",
    ] {
        assert!(
            !has_aarch64_packed_i32_add(disassembly),
            "unexpected packed integer ADD in {disassembly}"
        );
    }
}

fn has_llvm_fast_math_flags(llvm_ir: &str) -> bool {
    const FLAGS: [&str; 8] = [
        "fast", "nnan", "ninf", "nsz", "arcp", "contract", "afn", "reassoc",
    ];
    const FP_OPS: [&str; 16] = [
        "fneg", "fadd", "fsub", "fmul", "fdiv", "frem", "fcmp", "fptrunc", "fpext", "uitofp",
        "sitofp", "phi", "select", "call", "invoke", "callbr",
    ];
    llvm_ir.lines().any(|line| {
        let instruction = line
            .split_once(';')
            .map_or(line, |(instruction, _)| instruction)
            .trim();
        if instruction.is_empty() || instruction.starts_with('!') {
            return false;
        }

        let instruction = instruction
            .split_once('=')
            .map_or(instruction, |(_, instruction)| instruction.trim());
        let mut tokens = instruction.split_whitespace();
        while matches!(tokens.clone().next(), Some("tail" | "musttail" | "notail")) {
            tokens.next();
        }
        let Some(opcode) = tokens.next() else {
            return false;
        };
        FP_OPS.contains(&opcode) && tokens.next().is_some_and(|token| FLAGS.contains(&token))
    })
}

#[test]
fn strict_scalar_llvm_contract_rejects_fast_math_flags() {
    assert!(!has_llvm_fast_math_flags(
        "%negated = fneg double %value\n%quotient = fdiv double %negated, %divisor\n"
    ));
    assert!(!has_llvm_fast_math_flags(
        "%fast_value = fdiv double %left, %right\nret double %fast_value\n"
    ));
    let metadata_and_comment_only = concat!(
        "%negated = fneg double %value\n",
        "%quotient = fdiv double %negated, %divisor ; fast nnan ninf nsz arcp contract afn reassoc\n",
        "!0 = !{!\"no fast math nnan ninf nsz arcp contract afn reassoc\"}\n",
    );
    assert!(
        !has_llvm_fast_math_flags(metadata_and_comment_only),
        "metadata and comments must not be treated as instruction flags"
    );
    for flag in [
        "fast", "nnan", "ninf", "nsz", "arcp", "contract", "afn", "reassoc",
    ] {
        for llvm in [
            format!("%negated = fneg {flag} double %value\n"),
            format!("%quotient = fdiv {flag} double %left, %right\n"),
        ] {
            assert!(
                has_llvm_fast_math_flags(&llvm),
                "failed to recognize LLVM fast-math flag {flag}: {llvm}"
            );
        }
    }
}

#[test]
fn vector_loop_simd_should_survive_kir_llvm_and_object_code_on_the_pinned_host() {
    let target = NativeTarget::host_with_cpu(NativeCpu::Baseline).expect("baseline target");
    let profile = target
        .kir_profile(KirConsumer::NativeLibrary)
        .expect("queried target profile");
    #[cfg(target_arch = "x86_64")]
    assert!(
        profile.maximum_interleave_factor() >= 4,
        "x86 target profile must expose the closed four-chain frontier"
    );
    let checked = check(&SourceFile::new("loop-simd.ck", LOOP_SIMD_SOURCE));
    assert_eq!(checked.diagnostics, []);
    let mir = lower_to_mir(&checked.checked_program).expect("loop SIMD MIR");
    let kir = build_kir_module_with_profile(
        &mir,
        KirBuildConfig {
            consumer: KirConsumer::NativeLibrary,
            overflow_mode: KirOverflowMode::Unchecked,
            bounds_mode: KirBoundsMode::Unchecked,
            sanitizer_mode: KirSanitizerMode::Disabled,
        },
        profile.clone(),
    )
    .expect("loop SIMD KIR");
    let contracts =
        import_contract_facts(&kir, &checked.checked_program, 0).expect("loop SIMD contracts");
    let result = run_kir_pass_pipeline(kir, KirOptimizationLevel::O3, Some(&contracts));
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    assert_eq!(
        result.stats.vectorized_loops, 1,
        "{:?}",
        result.analysis_fallbacks
    );
    let plan_explanations = result
        .vector_explanations
        .iter()
        .map(|explanation| explanation.stable_text())
        .collect::<Vec<_>>()
        .join("\n");
    let accepted_plans = result
        .vector_explanations
        .iter()
        .filter(|explanation| explanation.disposition == calckernel::CandidateDisposition::Accepted)
        .collect::<Vec<_>>();
    assert_eq!(
        accepted_plans.len(),
        1,
        "expected exactly one accepted loop-vector plan:\n{plan_explanations}"
    );
    let accepted = accepted_plans[0];
    let selected_plan = accepted.stable_text();
    println!(
        "loop SIMD target profile: target={:?}, cpu={:?}, producer={:?}, consumer={:?}, layout={:?}, digest={}, max_interleave={}",
        profile.target_identity(),
        profile.cpu_identity(),
        profile.producer_identity(),
        profile.consumer(),
        profile.layout(),
        profile.digest_hex(),
        profile.maximum_interleave_factor(),
    );
    println!("selected loop SIMD plan: {selected_plan}");
    assert!(
        matches!(accepted.vf, 2 | 4 | 8 | 16)
            && profile.supports_vector_shape(KirLaneType::I32, accepted.vf),
        "the accepted VF must be in the closed frontier and legal for the target profile: {selected_plan}"
    );
    assert!(
        matches!(accepted.uf, 1 | 2 | 4) && accepted.uf <= profile.maximum_interleave_factor(),
        "the accepted UF must be in the closed frontier and legal for the target profile: {selected_plan}"
    );
    assert!(
        matches!(
            &accepted.candidate,
            calckernel::CandidateKey::LoopFrontier {
                kind: calckernel::LoopCandidateKind::LoopSimd,
                variant: calckernel::LoopCandidateVariant::Scalar,
                vf,
                uf,
                ..
            } if *vf == accepted.vf && *uf == accepted.uf
        ),
        "the accepted explanation must identify its matching scalar Loop SIMD candidate: {selected_plan}"
    );
    let kir_text = print_kir_module(result.artifact.as_ref().expect("vector artifact"));
    for spelling in [
        "loop_simd_body",
        "vector_load",
        "vector_add.modular",
        "vector_store",
    ] {
        assert!(
            kir_text.contains(spelling),
            "missing {spelling}:\n{kir_text}"
        );
    }

    let context = NativeContext::new().expect("context");
    let verified = lower_native_kir_module(&context, &target, &result, &EmitLlvmOptions::default())
        .expect("lower vector loop")
        .verify()
        .expect("verify vector loop");
    let pre_llvm = verified.to_ir_string().expect("pre-optimization LLVM IR");
    for spelling in [
        format!("load <{} x i32>", accepted.vf),
        format!("add <{} x i32>", accepted.vf),
        format!("store <{} x i32>", accepted.vf),
    ] {
        assert!(
            pre_llvm.contains(spelling.as_str()),
            "missing {spelling} for {selected_plan}:\n{pre_llvm}"
        );
    }
    let optimized = verified
        .audit()
        .expect("audit vector facts")
        .optimize(&target, NativeOptimizationLevel::O3)
        .expect("optimize vector loop");
    let optimized_ir = optimized
        .to_ir_string()
        .expect("print optimized vector loop");
    assert!(
        optimized_ir.contains("llvm.loop.unroll.disable"),
        "prevectorized KIR loop must retain its selected UF:\n{optimized_ir}"
    );
    let object = target.emit_object(optimized).expect("emit vector object");
    let root = std::env::temp_dir().join(format!("ckc-vector-object-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir(&root).expect("create vector object directory");
    let object_path = root.join("loop-simd.o");
    fs::write(&object_path, object.as_bytes()).expect("write vector object");
    let llvm_prefix = std::env::var("CKC_LLVM_PREFIX").expect("pinned LLVM prefix");
    let output = Command::new(std::path::Path::new(&llvm_prefix).join("bin/llvm-objdump"))
        .arg("-d")
        .arg(&object_path)
        .output()
        .expect("disassemble vector object");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let disassembly = String::from_utf8_lossy(&output.stdout).to_ascii_lowercase();
    let has_simd = if cfg!(target_arch = "aarch64") {
        has_aarch64_packed_i32_add(&disassembly)
    } else if cfg!(target_arch = "x86_64") {
        disassembly.contains("xmm") || disassembly.contains("ymm")
    } else {
        true
    };
    assert!(
        has_simd,
        "no host SIMD instruction matching the required object contract for {selected_plan}:\n{disassembly}"
    );
    fs::remove_dir_all(root).expect("remove vector object directory");
}

#[test]
fn vector_loop_simd_runtime_versioning_should_lower_a_total_address_predicate() {
    let target = NativeTarget::host_with_cpu(NativeCpu::Baseline).expect("baseline target");
    let profile = target
        .kir_profile(KirConsumer::NativeLibrary)
        .expect("queried target profile");
    let checked = check(&SourceFile::new(
        "versioned-loop-simd.ck",
        VERSIONED_LOOP_SIMD_SOURCE,
    ));
    assert_eq!(checked.diagnostics, []);
    let mir = lower_to_mir(&checked.checked_program).expect("versioned loop SIMD MIR");
    let kir = build_kir_module_with_profile(
        &mir,
        KirBuildConfig {
            consumer: KirConsumer::NativeLibrary,
            overflow_mode: KirOverflowMode::Unchecked,
            bounds_mode: KirBoundsMode::Unchecked,
            sanitizer_mode: KirSanitizerMode::Disabled,
        },
        profile,
    )
    .expect("versioned loop SIMD KIR");
    let contracts = import_contract_facts(&kir, &checked.checked_program, 0)
        .expect("versioned loop SIMD contracts");
    let result = run_kir_pass_pipeline(kir, KirOptimizationLevel::O3, Some(&contracts));
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    assert_eq!(
        result.stats.vectorized_loops, 1,
        "{:?}",
        result.analysis_fallbacks
    );
    let kir_text = print_kir_module(result.artifact.as_ref().expect("versioned artifact"));
    for spelling in ["version_predicate bits=64", "trip(", "disjoint("] {
        assert!(
            kir_text.contains(spelling),
            "missing {spelling}:\n{kir_text}"
        );
    }

    let context = NativeContext::new().expect("context");
    let llvm = lower_native_kir_module(&context, &target, &result, &EmitLlvmOptions::default())
        .expect("lower versioned vector loop")
        .verify()
        .expect("verify versioned vector loop")
        .to_ir_string()
        .expect("versioned LLVM IR");
    for spelling in [
        "ptrtoint",
        "version.trip",
        "version.disjoint",
        "version.total",
    ] {
        assert!(llvm.contains(spelling), "missing {spelling}:\n{llvm}");
    }
}

#[test]
fn vector_loop_simd_strict_f64_should_lower_without_fast_math_or_contraction() {
    let target = NativeTarget::host_with_cpu(NativeCpu::Baseline).expect("baseline target");
    let profile = target
        .kir_profile(KirConsumer::NativeLibrary)
        .expect("queried target profile");
    let checked = check(&SourceFile::new(
        "strict-f64-loop-simd.ck",
        STRICT_F64_LOOP_SIMD_SOURCE,
    ));
    assert_eq!(checked.diagnostics, []);
    let mir = lower_to_mir(&checked.checked_program).expect("strict f64 loop SIMD MIR");
    let kir = build_kir_module_with_profile(
        &mir,
        KirBuildConfig {
            consumer: KirConsumer::NativeLibrary,
            overflow_mode: KirOverflowMode::Unchecked,
            bounds_mode: KirBoundsMode::Unchecked,
            sanitizer_mode: KirSanitizerMode::Disabled,
        },
        profile,
    )
    .expect("strict f64 loop SIMD KIR");
    let contracts = import_contract_facts(&kir, &checked.checked_program, 0)
        .expect("strict f64 loop SIMD contracts");
    let result = run_kir_pass_pipeline(kir, KirOptimizationLevel::O3, Some(&contracts));
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    assert_eq!(
        result.stats.vectorized_loops, 1,
        "{:?}",
        result.analysis_fallbacks
    );
    #[cfg(target_arch = "x86_64")]
    {
        let accepted = result
            .vector_explanations
            .iter()
            .find(|explanation| {
                explanation.disposition == calckernel::CandidateDisposition::Accepted
            })
            .expect("accepted strict-f64 vector plan");
        assert_eq!(
            accepted.vf, 2,
            "x86 strict-f64 lowering must use f64 vectors"
        );
        assert!(
            accepted.uf >= 2 && accepted.uf <= 4,
            "x86 strict-f64 lowering must select a profitable independent-chain plan"
        );
    }
    let context = NativeContext::new().expect("context");
    let llvm = lower_native_kir_module(&context, &target, &result, &EmitLlvmOptions::default())
        .expect("lower strict f64 vector loop")
        .verify()
        .expect("verify strict f64 vector loop")
        .to_ir_string()
        .expect("strict f64 LLVM IR");
    let fmul = llvm
        .lines()
        .find(|line| line.contains("fmul <2 x double>"))
        .expect("fixed vector strict f64 multiply");
    assert!(
        !fmul.contains(" fast ") && !fmul.contains("contract"),
        "{fmul}"
    );
}

#[test]
fn vector_loop_simd_strict_f64_unary_divide_should_lower_without_reassociation() {
    let target = NativeTarget::host_with_cpu(NativeCpu::Baseline).expect("baseline target");
    let profile = target
        .kir_profile(KirConsumer::NativeLibrary)
        .expect("queried target profile");
    let checked = check(&SourceFile::new(
        "strict-f64-unary-divide-loop-simd.ck",
        STRICT_F64_UNARY_DIVIDE_LOOP_SIMD_SOURCE,
    ));
    assert_eq!(checked.diagnostics, []);
    let mir = lower_to_mir(&checked.checked_program).expect("strict f64 unary/divide SIMD MIR");
    let kir = build_kir_module_with_profile(
        &mir,
        KirBuildConfig {
            consumer: KirConsumer::NativeLibrary,
            overflow_mode: KirOverflowMode::Unchecked,
            bounds_mode: KirBoundsMode::Unchecked,
            sanitizer_mode: KirSanitizerMode::Disabled,
        },
        profile.clone(),
    )
    .expect("strict f64 unary/divide SIMD KIR");
    let contracts = import_contract_facts(&kir, &checked.checked_program, 0)
        .expect("strict f64 unary/divide SIMD contracts");
    let result = run_kir_pass_pipeline(kir, KirOptimizationLevel::O3, Some(&contracts));
    if cfg!(target_arch = "x86_64") {
        println!(
            "strict divide target profile: target={:?}, cpu={:?}, producer={:?}, consumer={:?}, layout={:?}, digest={}, max_interleave={}",
            profile.target_identity(),
            profile.cpu_identity(),
            profile.producer_identity(),
            profile.consumer(),
            profile.layout(),
            profile.digest_hex(),
            profile.maximum_interleave_factor(),
        );
        println!("strict divide pipeline errors: {:#?}", result.errors);
        println!(
            "strict divide analysis fallbacks: {:#?}",
            result.analysis_fallbacks
        );
        println!(
            "strict divide vector explanations: {:#?}",
            result.vector_explanations
        );
        println!(
            "strict divide optimizer audit attempts: {:#?}",
            result.audit.attempts()
        );
    }
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    let artifact = result
        .artifact
        .as_ref()
        .expect("strict f64 unary/divide optimized KIR artifact");
    let function_id = artifact
        .functions
        .iter()
        .find(|function| function.name == "map_f64_div")
        .expect("strict f64 unary/divide function")
        .id;
    if cfg!(target_arch = "x86_64") {
        let diagnostics = format!(
            "fallbacks={:#?}; explanations={:#?}; audit={:#?}",
            result.analysis_fallbacks,
            result.vector_explanations,
            result.audit.attempts()
        );
        assert_eq!(
            result.stats.vectorized_loops, 0,
            "{:?}",
            result.analysis_fallbacks
        );
        let loop_simd_fallbacks = result
            .analysis_fallbacks
            .iter()
            .filter(|fallback| fallback.function == function_id && fallback.pass == "loop-simd")
            .collect::<Vec<_>>();
        assert_eq!(
            loop_simd_fallbacks.len(),
            1,
            "expected one same-function Loop SIMD fallback; {diagnostics}"
        );
        assert_eq!(
            loop_simd_fallbacks[0].reason, "vector-chunk-benefit-not-met",
            "strict divide scalar fallback must come from the chunk-benefit cost gate; {diagnostics}"
        );

        let loop_simd_attempts = result
            .audit
            .attempts()
            .iter()
            .filter(|attempt| {
                matches!(
                    &attempt.key,
                    calckernel::CandidateKey::LoopFrontier {
                        function,
                        kind: calckernel::LoopCandidateKind::LoopSimd,
                        ..
                    } if *function == function_id
                )
            })
            .collect::<Vec<_>>();
        assert!(
            !loop_simd_attempts.is_empty(),
            "expected same-function candidate-keyed Loop SIMD audit attempts; {diagnostics}"
        );
        assert!(
            loop_simd_attempts.iter().all(|attempt| {
                attempt.disposition != calckernel::CandidateDisposition::Accepted
            }),
            "the strict divide fallback must not have an accepted Loop SIMD audit plan: {loop_simd_attempts:#?}"
        );
        assert!(
            loop_simd_attempts.iter().any(|attempt| {
                attempt.disposition == calckernel::CandidateDisposition::Rejected
                    && attempt.reason == "insufficient-benefit"
            }),
            "expected rejected same-function Loop SIMD candidates classified as insufficient-benefit; {diagnostics}"
        );
        assert!(
            result.vector_explanations.iter().all(|explanation| {
                explanation.disposition != calckernel::CandidateDisposition::Accepted
                    || !matches!(
                        &explanation.candidate,
                        calckernel::CandidateKey::LoopFrontier {
                            function,
                            kind: calckernel::LoopCandidateKind::LoopSimd,
                            ..
                        } if *function == function_id
                    )
            }),
            "strict divide scalar KIR unexpectedly has an accepted Loop SIMD explanation; {diagnostics}"
        );

        let kir = print_kir_module(artifact);
        assert!(!kir.contains("vector_divide"), "{kir}");
    } else {
        assert_eq!(
            result.stats.vectorized_loops, 1,
            "{:?}",
            result.analysis_fallbacks
        );
    }
    let context = NativeContext::new().expect("context");
    let verified = lower_native_kir_module(&context, &target, &result, &EmitLlvmOptions::default())
        .expect("lower strict f64 unary/divide vector loop")
        .verify()
        .expect("verify strict f64 unary/divide vector loop");
    let llvm = if cfg!(target_arch = "x86_64") {
        verified
            .audit()
            .expect("audit strict f64 scalar fallback facts")
            .to_ir_string()
            .expect("strict f64 audited scalar LLVM IR")
    } else {
        verified
            .to_ir_string()
            .expect("strict f64 unary/divide LLVM IR")
    };
    if cfg!(target_arch = "x86_64") {
        for operation in ["fneg double", "fdiv double"] {
            assert!(
                llvm.lines().any(|line| line.contains(operation)),
                "scalar strict divide lowering is missing {operation}:\n{llvm}"
            );
        }
        assert!(
            !llvm.contains("fdiv <"),
            "scalar strict divide lowering unexpectedly contains vector fdiv:\n{llvm}"
        );
        assert!(
            !has_llvm_fast_math_flags(&llvm),
            "scalar strict divide lowering contains a floating-point fast-math flag:\n{llvm}"
        );
    } else {
        for operation in ["fneg <2 x double>", "fdiv <2 x double>"] {
            let line = llvm
                .lines()
                .find(|line| line.contains(operation))
                .unwrap_or_else(|| panic!("missing {operation}:\n{llvm}"));
            assert!(
                !line.contains(" fast ") && !line.contains("contract"),
                "{line}"
            );
        }
    }
}

#[test]
fn vector_loop_simd_modular_reductions_should_survive_into_pre_llvm_ir() {
    let target = NativeTarget::host_with_cpu(NativeCpu::Baseline).expect("baseline target");
    let profile = target
        .kir_profile(KirConsumer::NativeLibrary)
        .expect("queried target profile");
    let checked = check(&SourceFile::new(
        "reduction-loop-simd.ck",
        REDUCTION_LOOP_SIMD_SOURCE,
    ));
    assert_eq!(checked.diagnostics, []);
    let mir = lower_to_mir(&checked.checked_program).expect("reduction loop SIMD MIR");
    let kir = build_kir_module_with_profile(
        &mir,
        KirBuildConfig {
            consumer: KirConsumer::NativeLibrary,
            overflow_mode: KirOverflowMode::Unchecked,
            bounds_mode: KirBoundsMode::Unchecked,
            sanitizer_mode: KirSanitizerMode::Disabled,
        },
        profile,
    )
    .expect("reduction loop SIMD KIR");
    let contracts = import_contract_facts(&kir, &checked.checked_program, 0)
        .expect("reduction loop SIMD contracts");
    let result = run_kir_pass_pipeline(kir, KirOptimizationLevel::O3, Some(&contracts));
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    let expected_vectorized_loops = if cfg!(target_arch = "x86_64") { 0 } else { 2 };
    assert_eq!(
        result.stats.vectorized_loops, expected_vectorized_loops,
        "{:?}",
        result.analysis_fallbacks
    );
    let kir_text = print_kir_module(result.artifact.as_ref().expect("reduction artifact"));
    if cfg!(target_arch = "x86_64") {
        assert!(
            !kir_text.contains("vector_reduce_modularadd")
                && !kir_text.contains("vector_reduce_modularmultiply"),
            "per-chunk x86 horizontal reduction blocks LLVM's loop-carried accumulator:\n{kir_text}"
        );
        assert!(result.analysis_fallbacks.iter().any(|fallback| {
            fallback.pass == "loop-simd"
                && fallback.reason == "x86-horizontal-reduction-deferred-to-native-loop-vectorizer"
        }));
    } else {
        assert!(
            kir_text.contains("vector_reduce_modularadd"),
            "missing modular add reduction:\n{kir_text}"
        );
        assert!(
            kir_text.contains("vector_reduce_modularmultiply"),
            "missing modular multiply reduction:\n{kir_text}"
        );
    }
    let context = NativeContext::new().expect("context");
    let llvm = lower_native_kir_module(&context, &target, &result, &EmitLlvmOptions::default())
        .expect("lower reduction vector loops")
        .verify()
        .expect("verify reduction vector loops")
        .to_ir_string()
        .expect("reduction LLVM IR");
    if cfg!(target_arch = "x86_64") {
        assert!(!llvm.contains("llvm.vector.reduce.add"), "{llvm}");
        assert!(!llvm.contains("llvm.vector.reduce.mul"), "{llvm}");
    } else {
        assert!(llvm.contains("llvm.vector.reduce.add"), "{llvm}");
        assert!(llvm.contains("llvm.vector.reduce.mul"), "{llvm}");
    }
}

#[test]
fn vector_loop_simd_cast_and_pure_diamond_should_survive_into_pre_llvm_ir() {
    let target = NativeTarget::host_with_cpu(NativeCpu::Baseline).expect("baseline target");
    let profile = target
        .kir_profile(KirConsumer::NativeLibrary)
        .expect("queried target profile");
    let checked = check(&SourceFile::new(
        "cast-diamond-loop-simd.ck",
        CAST_AND_DIAMOND_LOOP_SIMD_SOURCE,
    ));
    assert_eq!(checked.diagnostics, []);
    let mir = lower_to_mir(&checked.checked_program).expect("cast/diamond loop SIMD MIR");
    let kir = build_kir_module_with_profile(
        &mir,
        KirBuildConfig {
            consumer: KirConsumer::NativeLibrary,
            overflow_mode: KirOverflowMode::Unchecked,
            bounds_mode: KirBoundsMode::Unchecked,
            sanitizer_mode: KirSanitizerMode::Disabled,
        },
        profile,
    )
    .expect("cast/diamond loop SIMD KIR");
    let contracts = import_contract_facts(&kir, &checked.checked_program, 0)
        .expect("cast/diamond loop SIMD contracts");
    let result = run_kir_pass_pipeline(kir, KirOptimizationLevel::O3, Some(&contracts));
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    assert_eq!(
        result.stats.vectorized_loops, 2,
        "{:?}",
        result.analysis_fallbacks
    );
    #[cfg(target_arch = "x86_64")]
    {
        let artifact = result.artifact.as_ref().expect("cast/diamond artifact");
        let map_cast = artifact
            .functions
            .iter()
            .find(|function| function.name == "map_cast")
            .expect("map_cast function")
            .id;
        let accepted = result
            .vector_explanations
            .iter()
            .find(|explanation| {
                explanation.disposition == calckernel::CandidateDisposition::Accepted
                    && matches!(
                        explanation.candidate,
                        calckernel::CandidateKey::LoopFrontier { function, .. }
                            if function == map_cast
                    )
            })
            .expect("accepted x86 integer-cast plan");
        assert_eq!(accepted.vf, 2, "x86 integer cast must produce f64 vectors");
        assert!(
            accepted.uf >= 2 && accepted.uf <= 4,
            "x86 integer cast must select a profitable independent-chain plan"
        );
    }
    let kir_text = print_kir_module(result.artifact.as_ref().expect("cast/diamond artifact"));
    for spelling in ["vector_cast", "vector_compare", "vector_select"] {
        assert!(
            kir_text.contains(spelling),
            "missing {spelling}:\n{kir_text}"
        );
    }
    let context = NativeContext::new().expect("context");
    let llvm = lower_native_kir_module(&context, &target, &result, &EmitLlvmOptions::default())
        .expect("lower cast/diamond vector loops")
        .verify()
        .expect("verify cast/diamond vector loops")
        .to_ir_string()
        .expect("cast/diamond LLVM IR");
    for spelling in ["uitofp <2 x i32>", "icmp ult <4 x i32>", "select <4 x i1>"] {
        assert!(llvm.contains(spelling), "missing {spelling}:\n{llvm}");
    }
}

#[test]
fn schema_seven_vector_corpus_should_materialize_vectors_for_both_safety_modes() {
    let target = NativeTarget::host_with_cpu(NativeCpu::Baseline).expect("baseline target");
    for name in [
        "map_u32",
        "zip_u32",
        "strict_f64",
        "integer_cast",
        "modular_reduction",
        "slp_quad",
        "runtime_noalias",
        "specialized_length",
    ] {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("benches/oracles/fixtures")
            .join(format!("{name}.ck"));
        let source = fs::read_to_string(&path).expect("read schema 7 vector fixture");
        let checked = check(&SourceFile::new(path.display().to_string(), source));
        assert_eq!(checked.diagnostics, [], "{name}");
        let mir = lower_to_mir(&checked.checked_program).expect("vector corpus MIR");
        for safety in [false, true] {
            let kir = build_kir_module_with_profile(
                &mir,
                KirBuildConfig {
                    consumer: KirConsumer::NativeLibrary,
                    overflow_mode: if safety {
                        KirOverflowMode::Checked
                    } else {
                        KirOverflowMode::Unchecked
                    },
                    bounds_mode: if safety {
                        KirBoundsMode::Checked
                    } else {
                        KirBoundsMode::Unchecked
                    },
                    sanitizer_mode: KirSanitizerMode::Disabled,
                },
                target
                    .kir_profile(KirConsumer::NativeLibrary)
                    .expect("schema 7 profile"),
            )
            .expect("schema 7 KIR");
            let contracts = import_contract_facts(&kir, &checked.checked_program, 0)
                .expect("schema 7 contract facts");
            let result = run_kir_pass_pipeline(kir, KirOptimizationLevel::O3, Some(&contracts));
            assert!(
                result.errors.is_empty(),
                "{name}/{safety}: {:?}",
                result.errors
            );
            let text = print_kir_module(result.artifact.as_ref().expect("verified schema 7 KIR"));
            if safety {
                assert!(
                    text.contains("check_condition") || text.contains("guard"),
                    "{name}/checked must retain its scalar first-error safety boundary:\n{text}"
                );
            } else {
                let native_llvm_handoff = (name == "modular_reduction"
                    && cfg!(target_arch = "x86_64")
                    && result.analysis_fallbacks.iter().any(|fallback| {
                        fallback.reason
                            == "x86-horizontal-reduction-deferred-to-native-loop-vectorizer"
                    }))
                    || (name == "specialized_length"
                        && result.analysis_fallbacks.iter().any(|fallback| {
                            fallback.reason
                                == "constant-call-loop-deferred-to-native-loop-vectorizer"
                        }));
                assert!(
                    native_llvm_handoff || text.contains("vector_"),
                    "{name}/unchecked remained scalar without an audited Native LLVM handoff:\n{text}"
                );
                let header = emit_native_header(
                    result.artifact.as_ref().expect("vector header artifact"),
                    NativeHeaderMode::Dynamic,
                );
                assert!(
                    header.contains("kernel("),
                    "{name} header lost its public export"
                );
                assert!(
                    !header.contains("vector_"),
                    "KIR vectors must remain ABI-internal"
                );
            }
        }
    }
}
