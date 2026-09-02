use calckernel::{
    BlockId, FactId, FunctionId, InstructionId, KirArithmeticSemantics, KirBlock, KirBoundsMode,
    KirBuildConfig, KirConsumer, KirFailureKind, KirFunction, KirInstruction, KirInstructionKind,
    KirModule, KirOverflowMode, KirProfileLayout, KirResult, KirSanitizerMode, KirTargetIdentity,
    KirTargetProfile, KirTerminator, KirValueType, MemoryRegionId, MemoryVersionId,
    MirArtifactConsumer, MirPrimitiveTypeName, MirType, ProofId, SourceFile, ValueId,
    build_kir_module, check, lower_to_mir, prepare_artifact_for_consumer, print_kir_module,
    validate_kir_module,
};

use std::collections::BTreeSet;

fn config() -> KirBuildConfig {
    KirBuildConfig {
        consumer: KirConsumer::Inspection,
        overflow_mode: KirOverflowMode::Checked,
        bounds_mode: KirBoundsMode::Unchecked,
        sanitizer_mode: KirSanitizerMode::Disabled,
    }
}

fn build(source_text: &str, build_config: KirBuildConfig) -> KirModule {
    let checked = check(&SourceFile::new("kir.ck", source_text));
    assert_eq!(checked.diagnostics, []);
    let mir = lower_to_mir(&checked.checked_program).expect("MIR lowering");
    build_kir_module(&mir, build_config).expect("KIR construction")
}

#[test]
fn vector_type_scalar_migration_should_wrap_only_internal_ssa_values() {
    let module = build(
        "export fn add(left: i32, right: i32) -> i32 { return left + right; }",
        config(),
    );
    let function = &module.functions[0];
    assert_eq!(
        function.params[0].type_node,
        MirType::Primitive(MirPrimitiveTypeName::I32)
    );
    assert_eq!(
        function.return_type,
        MirType::Primitive(MirPrimitiveTypeName::I32)
    );
    for block in &function.blocks {
        for param in &block.params {
            assert!(matches!(param.type_node, KirValueType::Scalar(_)));
        }
        for instruction in &block.instructions {
            for result in &instruction.results {
                assert!(matches!(result.type_node, KirValueType::Scalar(_)));
            }
        }
    }
}

#[test]
fn profile_portable_consumers_should_have_complete_deterministic_schema_one_identity() {
    let cases = [
        (
            KirTargetProfile::inspection(),
            KirTargetIdentity::Inspection,
            KirProfileLayout::PortableUnknown,
        ),
        (
            KirTargetProfile::portable_c(),
            KirTargetIdentity::PortableC,
            KirProfileLayout::PortableUnknown,
        ),
        (
            KirTargetProfile::webassembly(),
            KirTargetIdentity::WebAssembly,
            KirProfileLayout::Known {
                pointer_width_bits: 32,
                little_endian: true,
            },
        ),
    ];
    for (profile, identity, layout) in cases {
        assert_eq!(profile.schema_version(), 1);
        assert_eq!(profile.target_identity(), &identity);
        assert_eq!(profile.layout(), layout);
        assert!(!profile.vector_operations_enabled());
        assert!(profile.cost_entry_count() > 100);
        assert_eq!(profile.canonical_bytes(), profile.canonical_bytes());
        assert_eq!(profile.digest_hex().len(), 64);
        assert!(
            profile
                .digest_hex()
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        );
        assert_eq!(profile.validate(), Ok(()));
    }
    assert_eq!(
        KirTargetProfile::inspection(),
        KirTargetProfile::inspection()
    );
    assert_ne!(
        KirTargetProfile::inspection().digest_hex(),
        KirTargetProfile::portable_c().digest_hex()
    );
    assert_ne!(
        KirTargetProfile::portable_c().digest_hex(),
        KirTargetProfile::webassembly().digest_hex()
    );
}

#[test]
fn profile_kir_v3_target_binding_should_print_and_reject_a_consumer_mismatch() {
    let module = build("export fn answer() -> i32 { return 42; }", config());
    let text = print_kir_module(&module);
    assert!(text.starts_with("kir-v3 consumer=inspection "), "{text}");
    assert!(text.contains("profile-schema=1"), "{text}");
    assert!(
        text.contains(&format!("profile-sha256={}", module.profile.digest_hex())),
        "{text}"
    );

    let mut mismatched = module;
    mismatched.profile = KirTargetProfile::portable_c();
    let errors = validate_kir_module(&mismatched).errors;
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert_eq!(
        errors[0].message,
        "KIR target profile consumer does not match module consumer"
    );
}

#[test]
fn kir_model_and_printer_should_include_complete_module_identity() {
    let module = KirModule {
        config: config(),
        profile: KirTargetProfile::inspection(),
        entry: None,
        structs: Vec::new(),
        tune_layout: None,
        functions: Vec::new(),
    };

    assert_eq!(
        print_kir_module(&module),
        format!(
            "kir-v3 consumer=inspection overflow=checked bounds=unchecked sanitizer=disabled profile-schema=1 profile-sha256={}\n",
            module.profile.digest_hex()
        )
    );
}

#[test]
fn kir_printer_should_be_byte_deterministic_for_identical_model() {
    let module = KirModule {
        config: config(),
        profile: KirTargetProfile::inspection(),
        entry: None,
        structs: Vec::new(),
        tune_layout: None,
        functions: Vec::new(),
    };
    let expected = print_kir_module(&module);

    for _ in 0..50 {
        assert_eq!(print_kir_module(&module), expected);
    }
}

#[test]
fn kir_model_should_use_typed_ids_and_print_explicit_ssa_definitions() {
    assert_eq!(FunctionId::from_index(7).index(), 7);
    assert_eq!(BlockId::from_index(7).index(), 7);
    assert_eq!(ValueId::from_index(7).index(), 7);
    assert_eq!(InstructionId::from_index(7).index(), 7);
    assert_eq!(MemoryRegionId::from_index(7).index(), 7);
    assert_eq!(MemoryVersionId::from_index(7).index(), 7);
    assert_eq!(FactId::from_index(7).index(), 7);
    assert_eq!(ProofId::from_index(7).index(), 7);

    let i32_type = MirType::Primitive(MirPrimitiveTypeName::I32);
    let module = KirModule {
        config: config(),
        profile: KirTargetProfile::inspection(),
        entry: None,
        structs: Vec::new(),
        tune_layout: None,
        functions: vec![KirFunction {
            id: FunctionId::from_index(0),
            name: "answer".to_string(),
            exported: true,
            params: Vec::new(),
            return_type: i32_type.clone(),
            regions: Vec::new(),
            initial_memory: Vec::new(),
            vector_regions: Vec::new(),
            blocks: vec![KirBlock {
                id: BlockId::from_index(0),
                label: "bb0".to_string(),
                params: Vec::new(),
                memory_params: Vec::new(),
                instructions: vec![KirInstruction {
                    id: InstructionId::from_index(0),
                    results: vec![KirResult {
                        value: ValueId::from_index(0),
                        type_node: i32_type.into(),
                    }],
                    kind: KirInstructionKind::ConstInt {
                        value: "42".to_string(),
                    },
                    memory: None,
                    effect: None,
                }],
                terminator: KirTerminator::Return {
                    value: Some(ValueId::from_index(0)),
                    memory: Vec::new(),
                    effect_order: 0,
                },
            }],
        }],
    };

    assert_eq!(
        print_kir_module(&module),
        format!(
            concat!(
                "kir-v3 consumer=inspection overflow=checked bounds=unchecked sanitizer=disabled profile-schema=1 profile-sha256={}\n",
                "\nexport fn f0 answer() -> i32 {{\n",
                "bb0 b0():\n",
                "  i0 v0: i32 = const_int 42\n",
                "  return v0 [effect 0]\n",
                "}}\n",
            ),
            module.profile.digest_hex()
        )
    );
}

#[test]
fn kir_builder_should_convert_straight_line_mutable_mir_values_to_single_definition_ssa() {
    let module = build(
        r#"
        export fn accumulate(a: i32, b: i32) -> i32 {
          let value: i32 = a + b;
          value = value + 1;
          return value;
        }
        "#,
        config(),
    );

    assert_eq!(validate_kir_module(&module).errors, []);
    let function = &module.functions[0];
    let mut definitions = BTreeSet::new();
    for param in &function.params {
        assert!(definitions.insert(param.value));
    }
    for block in &function.blocks {
        for param in &block.params {
            assert!(definitions.insert(param.value));
        }
        for instruction in &block.instructions {
            for result in &instruction.results {
                assert!(definitions.insert(result.value));
            }
        }
    }
    assert!(function.blocks[0].params.is_empty());
    assert!(
        function.blocks[0]
            .instructions
            .iter()
            .filter(|instruction| matches!(instruction.kind, KirInstructionKind::Binary { .. }))
            .count()
            >= 2
    );
}

#[test]
fn kir_builder_should_form_valid_block_parameter_ssa_for_control_flow() {
    let module = build(
        r#"
        export fn accumulate(n: u32) -> u32 {
          let i: u32 = 0;
          let total: u32 = 0;
          while i < n {
            i = i + 1;
            if i == 2 { continue; }
            if i == n { break; }
            total = total + i;
          }
          return total;
        }
        "#,
        config(),
    );

    assert_eq!(validate_kir_module(&module).errors, []);
    let function = &module.functions[0];
    assert!(function.blocks.len() > 4);
    assert!(function.blocks[0].params.is_empty());
    assert!(
        function.blocks[1..]
            .iter()
            .all(|block| !block.params.is_empty())
    );

    for block in &function.blocks {
        let edges = match &block.terminator {
            KirTerminator::Return { .. } => Vec::new(),
            KirTerminator::Jump { edge } => vec![edge],
            KirTerminator::Branch {
                then_edge,
                else_edge,
                ..
            } => vec![then_edge, else_edge],
        };
        for edge in edges {
            let target = function
                .blocks
                .iter()
                .find(|candidate| candidate.id == edge.target)
                .expect("edge target");
            assert_eq!(edge.args.len(), target.params.len());
            assert_eq!(edge.memory_args.len(), target.memory_params.len());
        }
    }
}

#[test]
fn mutation_dominance_and_scalar_phi_arity_should_be_rejected() {
    let module = build(
        r#"
        export fn choose(value: i32, flag: bool) -> i32 {
          let result: i32 = value;
          if flag { result = value + 1; } else { result = value - 1; }
          return result;
        }
        "#,
        config(),
    );

    let mut undefined = module.clone();
    let instruction = undefined.functions[0]
        .blocks
        .iter_mut()
        .flat_map(|block| &mut block.instructions)
        .find(|instruction| matches!(instruction.kind, KirInstructionKind::Binary { .. }))
        .expect("binary instruction");
    let KirInstructionKind::Binary { left, .. } = &mut instruction.kind else {
        unreachable!();
    };
    *left = ValueId::from_index(u32::MAX);
    assert_eq!(
        validate_kir_module(&undefined).errors[0].message,
        "value v4294967295 is not defined"
    );

    let mut wrong_arity = module;
    let edge = wrong_arity.functions[0]
        .blocks
        .iter_mut()
        .find_map(|block| match &mut block.terminator {
            KirTerminator::Jump { edge } if !edge.args.is_empty() => Some(edge),
            KirTerminator::Branch { then_edge, .. } if !then_edge.args.is_empty() => {
                Some(then_edge)
            }
            _ => None,
        })
        .expect("edge with block arguments");
    edge.args.pop();
    assert!(
        validate_kir_module(&wrong_arity).errors[0]
            .message
            .contains("block argument arity")
    );
}

#[test]
fn kir_reachability_should_prune_before_ssa_and_runtime_capability_checks() {
    let checked = check(&SourceFile::new(
        "kir.ck",
        r#"
        export fn quiet() -> i32 { return 7; }
        fn noisy() -> void { print_newline(); }
        fn main() -> i32 { noisy(); return 0; }
        "#,
    ));
    assert_eq!(checked.diagnostics, []);
    let mir = lower_to_mir(&checked.checked_program).expect("MIR lowering");

    let c_artifact = prepare_artifact_for_consumer(&mir, MirArtifactConsumer::C)
        .expect("unreachable runtime must not reject C");
    assert_eq!(
        c_artifact
            .functions
            .iter()
            .map(|function| function.name.as_str())
            .collect::<Vec<_>>(),
        vec!["quiet"]
    );
    let inspection = prepare_artifact_for_consumer(&mir, MirArtifactConsumer::Inspection)
        .expect("inspection permits reachable runtime");
    assert_eq!(
        inspection
            .functions
            .iter()
            .map(|function| function.name.as_str())
            .collect::<Vec<_>>(),
        vec!["quiet", "noisy", "main"]
    );

    let c_kir = build_kir_module(
        &mir,
        KirBuildConfig {
            consumer: KirConsumer::C,
            ..config()
        },
    )
    .expect("KIR must prune before translating runtime instructions");
    assert_eq!(c_kir.functions.len(), 1);
    assert_eq!(c_kir.functions[0].name, "quiet");
}

#[test]
fn kir_reachability_should_reject_reachable_unsupported_runtime_with_stable_path() {
    let checked = check(&SourceFile::new(
        "kir.ck",
        "export fn bad() -> void { print_newline(); }",
    ));
    assert_eq!(checked.diagnostics, []);
    let mir = lower_to_mir(&checked.checked_program).expect("MIR lowering");

    let error = prepare_artifact_for_consumer(&mir, MirArtifactConsumer::WebAssembly)
        .expect_err("reachable native runtime is unsupported");
    assert_eq!(
        error.message,
        "WebAssembly artifact root 'bad' reaches native-only runtime intrinsic 'print_newline' through bad."
    );
}

#[test]
fn kir_checked_builder_should_materialize_ordered_overflow_division_and_bounds_guards() {
    let source = r#"
        export fn kernel(
          data: ptr<i32>, items: slice<i32>, a: i32, b: i32,
          index: u32, start: u32, end: u32
        ) -> i32 {
          let sum: i32 = a + b;
          let quotient: i32 = sum / b;
          let value: i32 = items[index];
          let window: slice<i32> = items[start..end];
          data[0] = value;
          return quotient;
        }
    "#;
    let checked = build(
        source,
        KirBuildConfig {
            bounds_mode: KirBoundsMode::Checked,
            ..config()
        },
    );
    assert_eq!(validate_kir_module(&checked).errors, []);
    let instructions = checked.functions[0]
        .blocks
        .iter()
        .flat_map(|block| &block.instructions)
        .collect::<Vec<_>>();
    let guards = instructions
        .iter()
        .filter_map(|instruction| {
            let KirInstructionKind::Guard { failure, .. } = instruction.kind else {
                return None;
            };
            Some((
                failure,
                instruction.effect.as_ref().expect("guard effect").order,
            ))
        })
        .collect::<Vec<_>>();
    assert_eq!(
        guards
            .iter()
            .map(|(failure, _)| *failure)
            .collect::<Vec<_>>(),
        vec![
            KirFailureKind::Overflow,
            KirFailureKind::DivisionByZero,
            KirFailureKind::Overflow,
            KirFailureKind::OutOfBounds,
            KirFailureKind::OutOfBounds,
        ]
    );
    assert!(guards.windows(2).all(|pair| pair[0].1 < pair[1].1));

    let add = instructions
        .iter()
        .find(|instruction| {
            matches!(
                instruction.kind,
                KirInstructionKind::Binary {
                    op: calckernel::MirBinaryOp::Add,
                    semantics: KirArithmeticSemantics::Checked,
                    ..
                }
            )
        })
        .expect("checked add");
    assert_eq!(
        add.results.len(),
        2,
        "checked add must expose overflow condition"
    );

    let unchecked = build(
        source,
        KirBuildConfig {
            overflow_mode: KirOverflowMode::Unchecked,
            bounds_mode: KirBoundsMode::Unchecked,
            ..config()
        },
    );
    assert_eq!(validate_kir_module(&unchecked).errors, []);
    assert!(
        !unchecked.functions[0]
            .blocks
            .iter()
            .flat_map(|block| &block.instructions)
            .any(|instruction| matches!(instruction.kind, KirInstructionKind::Guard { .. }))
    );
    assert!(
        unchecked.functions[0]
            .blocks
            .iter()
            .flat_map(|block| &block.instructions)
            .filter(|instruction| matches!(instruction.kind, KirInstructionKind::Binary { .. }))
            .all(|instruction| matches!(
                instruction.kind,
                KirInstructionKind::Binary {
                    semantics: KirArithmeticSemantics::Modular,
                    ..
                }
            ))
    );
}

#[test]
fn kir_builder_should_reject_unsupported_consumer_modes_before_construction() {
    let checked = check(&SourceFile::new(
        "kir.ck",
        "export fn value(a: i32, b: i32) -> i32 { return a + b; }",
    ));
    assert_eq!(checked.diagnostics, []);
    let mir = lower_to_mir(&checked.checked_program).expect("MIR lowering");

    let error = build_kir_module(
        &mir,
        KirBuildConfig {
            consumer: KirConsumer::WebAssembly,
            overflow_mode: KirOverflowMode::Checked,
            bounds_mode: KirBoundsMode::Unchecked,
            sanitizer_mode: KirSanitizerMode::Disabled,
        },
    )
    .expect_err("WASM checked overflow is unsupported");
    assert_eq!(
        error.message,
        "WebAssembly KIR consumer does not support checked overflow mode."
    );
}

#[test]
fn kir_builder_should_thread_conservative_region_memory_ssa_through_joins() {
    let module = build(
        r#"
        export fn update(data: ptr<i32>, flag: bool) -> i32 {
          if flag { data[0] = 1; } else { data[0] = 2; }
          return data[0];
        }
        "#,
        KirBuildConfig {
            overflow_mode: KirOverflowMode::Unchecked,
            bounds_mode: KirBoundsMode::Unchecked,
            ..config()
        },
    );

    assert_eq!(validate_kir_module(&module).errors, []);
    let function = &module.functions[0];
    assert!(function.regions.len() >= 2);
    assert_eq!(function.initial_memory.len(), 1);
    assert!(
        function.blocks[1..]
            .iter()
            .all(|block| block.memory_params.len() == 1)
    );
    assert!(
        function
            .blocks
            .iter()
            .flat_map(|block| &block.instructions)
            .filter(|instruction| matches!(
                instruction.kind,
                KirInstructionKind::Load { .. } | KirInstructionKind::Store { .. }
            ))
            .all(|instruction| instruction.memory.is_some())
    );
}

#[test]
fn mutation_undefined_memory_version_should_be_rejected() {
    let mut module = build(
        r#"
        export fn read(data: ptr<i32>) -> i32 { return data[0]; }
        "#,
        KirBuildConfig {
            overflow_mode: KirOverflowMode::Unchecked,
            ..config()
        },
    );
    let memory = module.functions[0]
        .blocks
        .iter_mut()
        .flat_map(|block| &mut block.instructions)
        .find_map(|instruction| instruction.memory.as_mut())
        .expect("memory access");
    memory.input = MemoryVersionId::from_index(u32::MAX);

    assert_eq!(
        validate_kir_module(&module).errors[0].message,
        "memory version m4294967295 is not defined"
    );
}

#[test]
fn kir_builder_should_validate_short_circuit_control_flow() {
    let module = build(
        r#"
        export fn select(a: bool, b: bool, left: i32, right: i32) -> i32 {
          let result: i32 = right;
          if a && b { result = left; }
          return result;
        }
        "#,
        config(),
    );

    assert_eq!(validate_kir_module(&module).errors, []);
    assert!(
        module.functions[0]
            .blocks
            .iter()
            .filter(|block| matches!(block.terminator, KirTerminator::Branch { .. }))
            .count()
            >= 2
    );
}

#[test]
fn kir_printer_should_expose_region_and_memory_ssa_without_environment_data() {
    let source = r#"
        export fn update(data: ptr<i32>, value: i32) -> i32 {
          data[0] = value;
          return data[0];
        }
    "#;
    let module = build(
        source,
        KirBuildConfig {
            overflow_mode: KirOverflowMode::Unchecked,
            ..config()
        },
    );
    let expected = print_kir_module(&module);

    assert!(expected.contains("region r"), "{expected}");
    assert!(expected.contains("initial_memory r"), "{expected}");
    assert!(expected.contains("[memory r"), "{expected}");
    assert!(!expected.contains("kir.ck"), "{expected}");
    assert!(!expected.contains("/Users/"), "{expected}");
    for _ in 0..50 {
        assert_eq!(
            print_kir_module(&build(
                source,
                KirBuildConfig {
                    overflow_mode: KirOverflowMode::Unchecked,
                    ..config()
                },
            )),
            expected
        );
    }
}

#[test]
fn mutation_removed_required_checked_guard_should_be_rejected() {
    let mut module = build(
        "export fn add(a: i32, b: i32) -> i32 { return a + b; }",
        config(),
    );
    let block = &mut module.functions[0].blocks[0];
    let guard_index = block
        .instructions
        .iter()
        .position(|instruction| matches!(instruction.kind, KirInstructionKind::Guard { .. }))
        .expect("required overflow guard");
    block.instructions.remove(guard_index);

    assert!(validate_kir_module(&module).errors.iter().any(|error| {
        error
            .message
            .contains("checked arithmetic result is not followed by its required overflow guard")
    }));
}

#[test]
fn mutation_later_definition_used_early_should_be_rejected() {
    let mut module = build(
        r#"
        export fn calculate(a: i32, b: i32) -> i32 {
          let first: i32 = a + b;
          let second: i32 = first + 1;
          return second;
        }
        "#,
        config(),
    );
    let binaries = module.functions[0].blocks[0]
        .instructions
        .iter()
        .enumerate()
        .filter(|(_, instruction)| matches!(instruction.kind, KirInstructionKind::Binary { .. }))
        .map(|(index, instruction)| (index, instruction.results[0].value))
        .collect::<Vec<_>>();
    let later = binaries[1].1;
    let KirInstructionKind::Binary { left, .. } =
        &mut module.functions[0].blocks[0].instructions[binaries[0].0].kind
    else {
        unreachable!();
    };
    *left = later;

    assert!(validate_kir_module(&module).errors.iter().any(|error| {
        error.message
            == format!(
                "value v{} definition does not dominate its use",
                later.index()
            )
    }));
}

#[test]
fn mutation_invalid_region_partition_and_instruction_types_should_be_rejected() {
    let mut invalid_region = build(
        "export fn read(data: ptr<i32>) -> i32 { return data[0]; }",
        KirBuildConfig {
            overflow_mode: KirOverflowMode::Unchecked,
            ..config()
        },
    );
    invalid_region.functions[0].regions[1].partition = MemoryRegionId::from_index(u32::MAX);
    assert!(
        validate_kir_module(&invalid_region)
            .errors
            .iter()
            .any(|error| {
                error.message == "memory region r1 names undefined partition r4294967295"
            })
    );

    let mut invalid_type = build(
        "export fn add(a: i32, flag: bool) -> i32 { return a + 1; }",
        config(),
    );
    let bool_value = invalid_type.functions[0].params[1].value;
    let binary = invalid_type.functions[0].blocks[0]
        .instructions
        .iter_mut()
        .find(|instruction| matches!(instruction.kind, KirInstructionKind::Binary { .. }))
        .expect("binary");
    let KirInstructionKind::Binary { right, .. } = &mut binary.kind else {
        unreachable!();
    };
    *right = bool_value;
    assert!(
        validate_kir_module(&invalid_type)
            .errors
            .iter()
            .any(|error| {
                error.message == "binary operands and value result must have one type"
            })
    );
}

#[test]
fn kir_builder_should_preserve_ordered_runtime_and_may_fail_call_effects() {
    let module = build(
        r#"
        fn helper() -> void { print_newline(); }
        fn main() -> void { helper(); }
        "#,
        KirBuildConfig {
            consumer: KirConsumer::NativeExecutable,
            overflow_mode: KirOverflowMode::Unchecked,
            ..config()
        },
    );

    assert_eq!(validate_kir_module(&module).errors, []);
    let helper = module
        .functions
        .iter()
        .find(|function| function.name == "helper")
        .expect("helper");
    let runtime = helper.blocks[0]
        .instructions
        .iter()
        .find(|instruction| matches!(instruction.kind, KirInstructionKind::RuntimeCall { .. }))
        .expect("runtime call");
    assert_eq!(runtime.effect.as_ref().expect("runtime effect").order, 0);
    let main = module
        .functions
        .iter()
        .find(|function| function.name == "main")
        .expect("main");
    let call = main.blocks[0]
        .instructions
        .iter()
        .find(|instruction| matches!(instruction.kind, KirInstructionKind::Call { .. }))
        .expect("call");
    assert_eq!(call.effect.as_ref().expect("call effect").order, 0);
    let KirTerminator::Return { effect_order, .. } = main.blocks[0].terminator else {
        panic!("return");
    };
    assert_eq!(effect_order, 1);
}

#[test]
fn mutation_ordered_runtime_and_return_effects_should_be_rejected() {
    let mut module = build(
        "fn main() -> void { print_newline(); }",
        KirBuildConfig {
            consumer: KirConsumer::NativeExecutable,
            overflow_mode: KirOverflowMode::Unchecked,
            ..config()
        },
    );
    let block = &mut module
        .functions
        .iter_mut()
        .find(|function| function.name == "main")
        .expect("main")
        .blocks[0];
    let runtime_order = block.instructions[0]
        .effect
        .as_ref()
        .expect("runtime effect")
        .order;
    let KirTerminator::Return { effect_order, .. } = &mut block.terminator else {
        panic!("return")
    };
    *effect_order = runtime_order;

    assert!(validate_kir_module(&module).errors.iter().any(|error| {
        error
            .message
            .contains("ordered effect sequence must be strictly increasing")
    }));
}
