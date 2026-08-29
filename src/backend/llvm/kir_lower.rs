use std::collections::{BTreeMap, HashMap, HashSet};

use crate::*;

use super::{
    EmitLlvmOptions,
    abi::{add_export_thunks, implementation_name},
    builder::{NativeBlock, NativeBuilder, NativeFunction, NativeType, NativeValue},
    context::NativeContext,
    entry::add_entry_wrapper,
    error::NativeError,
    fact_audit::{NativeFactProperty, NativeFactSource, NativeStrengtheningKind},
    ffi::{BridgeCastOp, BridgeCompareOp, BridgeMemoryEffects, BridgeOverflowOp},
    layout::LlvmStructLayout,
    lower::{TypeRegistry, binary_op, compare_op, lowering_error, runtime_signature, unary_op},
    module::NativeModule,
    names::llvm_source_file_name,
    target::NativeTarget,
};

/// Lowers one optimized, evidence-verified Native KIR artifact directly to LLVM.
pub fn lower_native_kir_module<'context>(
    context: &'context NativeContext,
    target: &NativeTarget,
    result: &KirPassManagerResult,
    options: &EmitLlvmOptions,
) -> Result<NativeModule<'context>, NativeError> {
    if !result.errors.is_empty() {
        return Err(lowering_error(format!(
            "KIR pipeline is not verified: {}",
            result.errors.join("; ")
        )));
    }
    let kir = result
        .artifact
        .as_ref()
        .ok_or_else(|| lowering_error("KIR pipeline has no verified artifact"))?;
    if !matches!(
        kir.config.consumer,
        KirConsumer::NativeLibrary | KirConsumer::NativeExecutable
    ) {
        return Err(lowering_error(
            "native LLVM lowering requires a native KIR consumer",
        ));
    }
    if let Some(requested) = options.target_triple.as_deref() {
        let actual = target.triple()?;
        if requested != actual {
            return Err(lowering_error(format!(
                "requested target triple '{requested}' does not match native target '{actual}'"
            )));
        }
    }

    let shape = mir_shape(kir);
    let mut module = NativeModule::empty(context)?;
    module.configure(
        target,
        &llvm_source_file_name(options.source_file_name.as_deref()),
    )?;
    let wrap_proofs = collect_wrap_proofs(kir, result);
    let contract_attributes = collect_contract_attributes(kir, result);
    let contract_assumes = collect_contract_assumes(kir, result);
    let contract_memory_effects = collect_contract_memory_effects(kir, result);
    let scoped_alias_facts = collect_scoped_alias_facts(kir, result);
    for ((function, instruction), (proof, kind)) in &wrap_proofs {
        let function_name = kir
            .functions
            .iter()
            .find(|candidate| candidate.id == *function)
            .map(|candidate| candidate.name.clone())
            .ok_or_else(|| lowering_error("wrap proof names an unknown KIR function"))?;
        module.register_fact_property(NativeFactProperty {
            kind: *kind,
            source: NativeFactSource::Proof(*proof),
            function: function_name,
            subject: format!("i{}", instruction.index()),
        });
    }
    for attribute in contract_attributes.values().flatten() {
        module.register_fact_property(attribute.property.clone());
    }
    for (function, assumes) in &contract_assumes {
        let function_name = kir
            .functions
            .iter()
            .find(|candidate| candidate.id == *function)
            .map(|candidate| candidate.name.clone())
            .ok_or_else(|| lowering_error("contract assume names an unknown KIR function"))?;
        for assumption in assumes {
            for kind in [
                NativeStrengtheningKind::Range,
                NativeStrengtheningKind::Assume,
            ] {
                module.register_fact_property(NativeFactProperty {
                    kind,
                    source: NativeFactSource::Fact(assumption.fact),
                    function: function_name.clone(),
                    subject: format!("contract.fact{}", assumption.fact.index()),
                });
            }
        }
    }
    for effect in contract_memory_effects.values() {
        module.register_fact_property(effect.property.clone());
    }
    for (function, facts) in &scoped_alias_facts {
        let function_name = kir
            .functions
            .iter()
            .find(|candidate| candidate.id == *function)
            .map(|candidate| candidate.name.clone())
            .ok_or_else(|| lowering_error("alias fact names an unknown KIR function"))?;
        for (fact, left, right) in facts {
            module.register_fact_property(NativeFactProperty {
                kind: NativeStrengtheningKind::AliasScope,
                source: NativeFactSource::Fact(*fact),
                function: function_name.clone(),
                subject: format!("v{}<->v{}", left.index(), right.index()),
            });
        }
    }
    let lowering_facts = NativeKirFacts {
        wrap_proofs: &wrap_proofs,
        contract_assumes: &contract_assumes,
        scoped_alias_facts: &scoped_alias_facts,
    };
    {
        let types = TypeRegistry::new(context, &shape)?;
        let status = status_abi(kir);
        let mut functions = HashMap::new();
        for (kir_function, mir_function) in kir.functions.iter().zip(&shape.functions) {
            let mut params = physical_param_types(&types, &kir_function.params)?;
            if status && kir_function.return_type != MirType::Void {
                params.push(types.pointer);
            }
            let implementation = if kir.config.consumer == KirConsumer::NativeExecutable
                && kir
                    .entry
                    .as_ref()
                    .is_some_and(|entry| entry.function_name == kir_function.name)
            {
                "__ck_user_main".to_string()
            } else {
                implementation_name(mir_function)
            };
            let handle = module.add_function(
                &implementation,
                if status {
                    types.i32
                } else {
                    types.get(&kir_function.return_type)?
                },
                &params,
                false,
            )?;
            for attribute in contract_attributes
                .get(&kir_function.id)
                .into_iter()
                .flatten()
            {
                apply_param_attribute(handle, attribute)?;
            }
            if let Some(effect) = contract_memory_effects.get(&kir_function.id) {
                handle.set_memory_effects(effect.effects)?;
            }
            functions.insert(kir_function.name.clone(), handle);
        }
        if let Some(entry) = &kir.entry {
            module.preserve_function(require_function(&functions, &entry.function_name)?)?;
        }
        for intrinsic in used_runtime_intrinsics(kir) {
            let (name, parameter) = runtime_signature(intrinsic);
            let params = parameter
                .as_ref()
                .map(|type_node| types.get(type_node))
                .transpose()?
                .into_iter()
                .collect::<Vec<_>>();
            functions.insert(
                name.to_string(),
                module.add_function(name, types.void, &params, true)?,
            );
        }
        let layout = LlvmStructLayout::new(&shape);
        let environment = KirLoweringEnvironment {
            types: &types,
            functions: &functions,
            layout: &layout,
            status_abi: status,
            facts: &lowering_facts,
        };
        for function in &kir.functions {
            lower_function(context, &module, function, &environment)?;
        }
        add_export_thunks(context, &module, target, &shape, &types, &functions, status)?;
        if kir.config.consumer == KirConsumer::NativeExecutable {
            add_entry_wrapper(context, &module, &shape, &types, &functions, status)?;
        }
    }
    Ok(module)
}

struct NativeKirFacts<'a> {
    wrap_proofs: &'a WrapProofMap,
    contract_assumes: &'a BTreeMap<FunctionId, Vec<ContractAssume>>,
    scoped_alias_facts: &'a ScopedAliasFactMap,
}

struct KirLoweringEnvironment<'module, 'context, 'a> {
    types: &'a TypeRegistry<'context>,
    functions: &'a HashMap<String, NativeFunction<'module>>,
    layout: &'a LlvmStructLayout,
    status_abi: bool,
    facts: &'a NativeKirFacts<'a>,
}

type ScopedAliasFactMap = BTreeMap<FunctionId, Vec<(FactId, ValueId, ValueId)>>;

fn collect_scoped_alias_facts(
    module: &KirModule,
    result: &KirPassManagerResult,
) -> ScopedAliasFactMap {
    let Some(contracts) = result.contract_facts.as_ref() else {
        return BTreeMap::new();
    };
    let mut collected = BTreeMap::new();
    for function in &module.functions {
        let params = function
            .params
            .iter()
            .map(|param| param.value)
            .collect::<HashSet<_>>();
        let facts = contracts
            .facts()
            .facts()
            .iter()
            .filter(|fact| matches!(fact.scope, FactScope::FunctionEntry(id) if id == function.id))
            .filter_map(|fact| match &fact.predicate {
                FactPredicate::Contract(ContractFactPredicate::NoAlias { left, right })
                    if params.contains(left) && params.contains(right) =>
                {
                    Some((fact.id, *left, *right))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        if !facts.is_empty() {
            collected.insert(function.id, facts);
        }
    }
    collected
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FunctionMemoryEffect {
    effects: BridgeMemoryEffects,
    property: NativeFactProperty,
}

fn collect_contract_memory_effects(
    module: &KirModule,
    result: &KirPassManagerResult,
) -> BTreeMap<FunctionId, FunctionMemoryEffect> {
    let Some(contracts) = result.contract_facts.as_ref() else {
        return BTreeMap::new();
    };
    let mut collected = BTreeMap::new();
    for function in &module.functions {
        if (status_abi(module) && function.return_type != MirType::Void)
            || function
                .blocks
                .iter()
                .flat_map(|block| &block.instructions)
                .any(|instruction| {
                    matches!(
                        instruction.kind,
                        KirInstructionKind::Call { .. } | KirInstructionKind::RuntimeCall { .. }
                    )
                })
        {
            continue;
        }
        let candidate = contracts.facts().facts().iter().find_map(|fact| {
            if !matches!(fact.scope, FactScope::FunctionEntry(id) if id == function.id) {
                return None;
            }
            let FactPredicate::Contract(ContractFactPredicate::EffectCeiling { is_none, items }) =
                &fact.predicate
            else {
                return None;
            };
            let effects = if *is_none
                || items
                    .iter()
                    .all(|(_, effect)| *effect == ContractEffectKind::None)
            {
                BridgeMemoryEffects::None
            } else if items
                .iter()
                .all(|(_, effect)| *effect == ContractEffectKind::Read)
            {
                BridgeMemoryEffects::Read
            } else if items
                .iter()
                .all(|(_, effect)| *effect == ContractEffectKind::Write)
            {
                BridgeMemoryEffects::Write
            } else {
                return None;
            };
            Some(FunctionMemoryEffect {
                effects,
                property: NativeFactProperty {
                    kind: NativeStrengtheningKind::MemoryEffects,
                    source: NativeFactSource::Fact(fact.id),
                    function: function.name.clone(),
                    subject: "function memory effects".to_string(),
                },
            })
        });
        if let Some(candidate) = candidate {
            collected.insert(function.id, candidate);
        }
    }
    collected
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum AssumeOperand {
    Value(ValueId),
    SliceLength(ValueId),
    Constant(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ContractAssume {
    fact: FactId,
    op: MirCompareOp,
    left: AssumeOperand,
    right: AssumeOperand,
    type_node: MirType,
}

fn collect_contract_assumes(
    module: &KirModule,
    result: &KirPassManagerResult,
) -> BTreeMap<FunctionId, Vec<ContractAssume>> {
    let Some(contracts) = result.contract_facts.as_ref() else {
        return BTreeMap::new();
    };
    let mut collected = BTreeMap::new();
    for function in &module.functions {
        let types = value_types(function);
        let assumptions = contracts
            .facts()
            .facts()
            .iter()
            .filter(|fact| matches!(fact.scope, FactScope::FunctionEntry(id) if id == function.id))
            .filter_map(|fact| {
                let FactPredicate::Contract(ContractFactPredicate::Comparison {
                    operator,
                    left,
                    right,
                }) = &fact.predicate
                else {
                    return None;
                };
                let left = assume_operand(left)?;
                let right = assume_operand(right)?;
                let left_type = assume_operand_type(&left, &types);
                let right_type = assume_operand_type(&right, &types);
                let type_node = left_type.or(right_type)?;
                if left_type.is_some_and(|candidate| candidate != type_node)
                    || right_type.is_some_and(|candidate| candidate != type_node)
                    || !matches!(
                        type_node,
                        MirType::Primitive(
                            MirPrimitiveTypeName::I32
                                | MirPrimitiveTypeName::I64
                                | MirPrimitiveTypeName::U32
                                | MirPrimitiveTypeName::U64
                        )
                    )
                {
                    return None;
                }
                Some(ContractAssume {
                    fact: fact.id,
                    op: comparison_operator(operator)?,
                    left,
                    right,
                    type_node: type_node.clone(),
                })
            })
            .collect::<Vec<_>>();
        if !assumptions.is_empty() {
            collected.insert(function.id, assumptions);
        }
    }
    collected
}

fn assume_operand(expression: &ContractFactAffineExpression) -> Option<AssumeOperand> {
    match expression.terms.as_slice() {
        [] => Some(AssumeOperand::Constant(expression.constant.to_string())),
        [term] if term.coefficient == 1.into() && expression.constant == 0.into() => {
            Some(match term.term {
                ContractFactAffineTerm::Value(value) => AssumeOperand::Value(value),
                ContractFactAffineTerm::SliceLength(value) => AssumeOperand::SliceLength(value),
            })
        }
        _ => None,
    }
}

fn assume_operand_type<'a>(
    operand: &AssumeOperand,
    types: &'a BTreeMap<ValueId, MirType>,
) -> Option<&'a MirType> {
    match operand {
        AssumeOperand::Value(value) => types.get(value),
        AssumeOperand::SliceLength(_) => Some(&MirType::Primitive(MirPrimitiveTypeName::U32)),
        AssumeOperand::Constant(_) => None,
    }
}

fn comparison_operator(operator: &str) -> Option<MirCompareOp> {
    match operator {
        "==" => Some(MirCompareOp::Eq),
        "!=" => Some(MirCompareOp::Ne),
        "<" => Some(MirCompareOp::Lt),
        "<=" => Some(MirCompareOp::Le),
        ">" => Some(MirCompareOp::Gt),
        ">=" => Some(MirCompareOp::Ge),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ParamAttributeKind {
    NoAlias,
    ReadOnly,
    WriteOnly,
    Alignment(u32),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParamFactAttribute {
    physical_index: usize,
    attribute: ParamAttributeKind,
    property: NativeFactProperty,
}

fn apply_param_attribute(
    function: NativeFunction<'_>,
    attribute: &ParamFactAttribute,
) -> Result<(), NativeError> {
    match attribute.attribute {
        ParamAttributeKind::NoAlias => function.add_param_noalias(attribute.physical_index),
        ParamAttributeKind::ReadOnly => function.add_param_readonly(attribute.physical_index),
        ParamAttributeKind::WriteOnly => function.add_param_writeonly(attribute.physical_index),
        ParamAttributeKind::Alignment(alignment) => {
            function.add_param_alignment(attribute.physical_index, alignment)
        }
    }
}

fn collect_contract_attributes(
    module: &KirModule,
    result: &KirPassManagerResult,
) -> BTreeMap<FunctionId, Vec<ParamFactAttribute>> {
    let Some(contracts) = result.contract_facts.as_ref() else {
        return BTreeMap::new();
    };
    let mut collected = BTreeMap::new();
    for function in &module.functions {
        let entry_facts = contracts
            .facts()
            .facts()
            .iter()
            .filter(|fact| matches!(fact.scope, FactScope::FunctionEntry(id) if id == function.id))
            .collect::<Vec<_>>();
        let pointer_params = function
            .params
            .iter()
            .filter(|param| is_pointer_like(&param.type_node))
            .map(|param| param.value)
            .collect::<Vec<_>>();
        let noalias_facts = entry_facts
            .iter()
            .filter_map(|fact| match &fact.predicate {
                FactPredicate::Contract(ContractFactPredicate::NoAlias { left, right }) => {
                    Some((fact.id, normalized_pair(*left, *right)))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        let complete_noalias = pointer_params.len() == 2
            && pointer_params.iter().enumerate().all(|(left_index, left)| {
                pointer_params.iter().skip(left_index + 1).all(|right| {
                    let pair = normalized_pair(*left, *right);
                    noalias_facts
                        .iter()
                        .any(|(_, candidate)| *candidate == pair)
                })
            })
            && !is_pointer_like(&function.return_type)
            && !(status_abi(module) && function.return_type != MirType::Void)
            && !function
                .blocks
                .iter()
                .flat_map(|block| &block.instructions)
                .any(|instruction| {
                    matches!(
                        instruction.kind,
                        KirInstructionKind::Call { .. } | KirInstructionKind::RuntimeCall { .. }
                    )
                });
        let mut attributes = Vec::new();
        if complete_noalias {
            for value in &pointer_params {
                if let (Some(index), Some((source, _))) = (
                    physical_parameter_index(function, *value),
                    noalias_facts
                        .iter()
                        .find(|(_, pair)| pair.0 == *value || pair.1 == *value),
                ) {
                    attributes.push(ParamFactAttribute {
                        physical_index: index,
                        attribute: ParamAttributeKind::NoAlias,
                        property: NativeFactProperty {
                            kind: NativeStrengtheningKind::ParameterNoAlias,
                            source: NativeFactSource::Fact(*source),
                            function: function.name.clone(),
                            subject: parameter_subject(function, *value),
                        },
                    });
                }
            }
        }
        for fact in &entry_facts {
            match &fact.predicate {
                FactPredicate::Contract(ContractFactPredicate::Aligned { pointer, alignment }) => {
                    let value = match pointer {
                        ContractFactPointer::Value(value)
                        | ContractFactPointer::SliceData(value) => *value,
                    };
                    if let Some(index) = physical_parameter_index(function, value) {
                        attributes.push(ParamFactAttribute {
                            physical_index: index,
                            attribute: ParamAttributeKind::Alignment(*alignment),
                            property: NativeFactProperty {
                                kind: NativeStrengtheningKind::Alignment,
                                source: NativeFactSource::Fact(fact.id),
                                function: function.name.clone(),
                                subject: parameter_subject(function, value),
                            },
                        });
                    }
                }
                FactPredicate::Contract(ContractFactPredicate::EffectCeiling { items, .. }) => {
                    for (value, effect) in items {
                        let Some(index) = physical_parameter_index(function, *value) else {
                            continue;
                        };
                        let (attribute, kind) = match effect {
                            ContractEffectKind::Read => (
                                ParamAttributeKind::ReadOnly,
                                NativeStrengtheningKind::ReadOnly,
                            ),
                            ContractEffectKind::Write => (
                                ParamAttributeKind::WriteOnly,
                                NativeStrengtheningKind::WriteOnly,
                            ),
                            ContractEffectKind::None | ContractEffectKind::ReadWrite => continue,
                        };
                        attributes.push(ParamFactAttribute {
                            physical_index: index,
                            attribute,
                            property: NativeFactProperty {
                                kind,
                                source: NativeFactSource::Fact(fact.id),
                                function: function.name.clone(),
                                subject: parameter_subject(function, *value),
                            },
                        });
                    }
                }
                _ => {}
            }
        }
        if !attributes.is_empty() {
            collected.insert(function.id, attributes);
        }
    }
    collected
}

fn normalized_pair(left: ValueId, right: ValueId) -> (ValueId, ValueId) {
    if left <= right {
        (left, right)
    } else {
        (right, left)
    }
}

fn is_pointer_like(type_node: &MirType) -> bool {
    matches!(type_node, MirType::Pointer(_) | MirType::Slice(_))
}

fn physical_parameter_index(function: &KirFunction, value: ValueId) -> Option<usize> {
    let mut physical = 0;
    for param in &function.params {
        if param.value == value {
            return is_pointer_like(&param.type_node).then_some(physical);
        }
        physical += usize::from(matches!(param.type_node, MirType::Slice(_))) + 1;
    }
    None
}

fn parameter_subject(function: &KirFunction, value: ValueId) -> String {
    function
        .params
        .iter()
        .find(|param| param.value == value)
        .map_or_else(|| format!("v{}", value.index()), |param| param.name.clone())
}

type WrapProofMap = BTreeMap<(FunctionId, InstructionId), (ProofId, NativeStrengtheningKind)>;

fn collect_wrap_proofs(module: &KirModule, result: &KirPassManagerResult) -> WrapProofMap {
    let mut proofs = BTreeMap::new();
    for elimination in &result.eliminated_guards {
        let Some(proof) = elimination.proof else {
            continue;
        };
        let Some(instruction) = module
            .functions
            .iter()
            .find(|function| function.id == elimination.function)
            .into_iter()
            .flat_map(|function| &function.blocks)
            .flat_map(|block| &block.instructions)
            .find(|instruction| instruction.id == elimination.condition_instruction)
        else {
            continue;
        };
        let KirInstructionKind::Binary {
            op: MirBinaryOp::Add | MirBinaryOp::Sub | MirBinaryOp::Mul,
            semantics: KirArithmeticSemantics::Checked,
            ..
        } = instruction.kind
        else {
            continue;
        };
        let kind = match instruction.results.first().map(|result| &result.type_node) {
            Some(MirType::Primitive(MirPrimitiveTypeName::U32 | MirPrimitiveTypeName::U64)) => {
                NativeStrengtheningKind::NoUnsignedWrap
            }
            Some(MirType::Primitive(MirPrimitiveTypeName::I32 | MirPrimitiveTypeName::I64)) => {
                NativeStrengtheningKind::NoSignedWrap
            }
            _ => continue,
        };
        proofs.insert(
            (elimination.function, elimination.condition_instruction),
            (proof, kind),
        );
    }
    proofs
}

fn status_abi(module: &KirModule) -> bool {
    module.config.overflow_mode == KirOverflowMode::Checked
        || module.config.bounds_mode == KirBoundsMode::Checked
}

fn mir_shape(module: &KirModule) -> MirModule {
    MirModule {
        entry: module.entry.clone(),
        structs: module.structs.clone(),
        functions: module
            .functions
            .iter()
            .map(|function| MirFunction {
                name: function.name.clone(),
                exported: function.exported,
                params: function
                    .params
                    .iter()
                    .map(|param| MirParam {
                        name: param.name.clone(),
                        type_node: param.type_node.clone(),
                    })
                    .collect(),
                return_type: function.return_type.clone(),
                locals: Vec::new(),
                blocks: Vec::new(),
            })
            .collect(),
    }
}

fn physical_param_types<'context>(
    types: &TypeRegistry<'context>,
    params: &[KirParam],
) -> Result<Vec<NativeType<'context>>, NativeError> {
    let mut physical = Vec::new();
    for param in params {
        if matches!(param.type_node, MirType::Slice(_)) {
            physical.extend([types.pointer, types.i32]);
        } else {
            physical.push(types.get(&param.type_node)?);
        }
    }
    Ok(physical)
}

fn used_runtime_intrinsics(module: &KirModule) -> Vec<MirRuntimeIntrinsic> {
    let mut seen = HashSet::new();
    module
        .functions
        .iter()
        .flat_map(|function| &function.blocks)
        .flat_map(|block| &block.instructions)
        .filter_map(|instruction| match instruction.kind {
            KirInstructionKind::RuntimeCall { intrinsic, .. } if seen.insert(intrinsic) => {
                Some(intrinsic)
            }
            _ => None,
        })
        .collect()
}

#[derive(Clone, Copy)]
struct Storage<'module> {
    pointer: NativeValue<'module>,
}

struct KirFunctionLowerer<'module, 'context, 'a> {
    builder: NativeBuilder<'module, 'context>,
    types: &'a TypeRegistry<'context>,
    functions: &'a HashMap<String, NativeFunction<'module>>,
    layout: &'a LlvmStructLayout,
    handle: NativeFunction<'module>,
    function: &'a KirFunction,
    status_abi: bool,
    result_pointer: Option<NativeValue<'module>>,
    blocks: BTreeMap<BlockId, NativeBlock<'module>>,
    current_block: Option<NativeBlock<'module>>,
    storage: BTreeMap<ValueId, Storage<'module>>,
    guard_conditions: HashSet<ValueId>,
    facts: &'a NativeKirFacts<'a>,
    temporary: u32,
}

fn lower_function<'module, 'context>(
    context: &'context NativeContext,
    module: &'module NativeModule<'context>,
    function: &KirFunction,
    environment: &KirLoweringEnvironment<'module, 'context, '_>,
) -> Result<(), NativeError> {
    let handle = require_function(environment.functions, &function.name)?;
    let entry = handle.append_block("entry")?;
    let blocks = function
        .blocks
        .iter()
        .map(|block| {
            handle
                .append_block(&format!("kir.bb{}", block.id.index()))
                .map(|native| (block.id, native))
        })
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    let mut lowerer = KirFunctionLowerer {
        builder: NativeBuilder::new(context, module)?,
        types: environment.types,
        functions: environment.functions,
        layout: environment.layout,
        handle,
        function,
        status_abi: environment.status_abi,
        result_pointer: None,
        blocks,
        current_block: Some(entry),
        storage: BTreeMap::new(),
        guard_conditions: function
            .blocks
            .iter()
            .flat_map(|block| &block.instructions)
            .filter_map(|instruction| match instruction.kind {
                KirInstructionKind::Guard { condition, .. } => Some(condition),
                _ => None,
            })
            .collect(),
        facts: environment.facts,
        temporary: 0,
    };
    lowerer.builder.position(entry)?;
    lowerer.allocate_values()?;
    lowerer.store_parameters()?;
    lowerer.emit_contract_assumes()?;
    let Some(first) = function.blocks.first() else {
        return if lowerer.status_abi {
            let ok = lowerer.status(0)?;
            lowerer.builder.return_value(ok)
        } else if function.return_type == MirType::Void {
            lowerer.builder.return_void()
        } else {
            Err(lowering_error(format!(
                "non-void KIR function '{}' has no blocks",
                function.name
            )))
        };
    };
    lowerer.builder.branch(lowerer.block(first.id)?)?;
    for block in &function.blocks {
        let native_block = lowerer.block(block.id)?;
        lowerer.builder.position(native_block)?;
        lowerer.current_block = Some(native_block);
        for instruction in &block.instructions {
            lowerer.instruction(instruction)?;
        }
        lowerer.terminator(block.id, &block.terminator)?;
    }
    Ok(())
}

impl<'module, 'context> KirFunctionLowerer<'module, 'context, '_> {
    fn name(&mut self, prefix: &str) -> String {
        let name = format!("{prefix}.{}", self.temporary);
        self.temporary += 1;
        name
    }

    fn block(&self, id: BlockId) -> Result<NativeBlock<'module>, NativeError> {
        self.blocks
            .get(&id)
            .copied()
            .ok_or_else(|| lowering_error(format!("unknown KIR block b{}", id.index())))
    }

    fn allocate_values(&mut self) -> Result<(), NativeError> {
        for (value, type_node) in value_types(self.function) {
            let pointer = self.builder.alloca(
                self.types.get(&type_node)?,
                &format!("v{}.addr", value.index()),
            )?;
            self.storage.insert(value, Storage { pointer });
        }
        Ok(())
    }

    fn store_parameters(&mut self) -> Result<(), NativeError> {
        let mut physical = 0;
        for param in &self.function.params {
            if matches!(param.type_node, MirType::Slice(_)) {
                let data = self
                    .handle
                    .param(physical, &format!("{}.data", param.name))?;
                let len = self
                    .handle
                    .param(physical + 1, &format!("{}.len", param.name))?;
                physical += 2;
                let slice = self.make_slice(data, len)?;
                self.store(param.value, slice)?;
            } else {
                let value = self.handle.param(physical, &param.name)?;
                physical += 1;
                self.store(param.value, value)?;
            }
        }
        if self.status_abi && self.function.return_type != MirType::Void {
            let result = self.handle.param(physical, "ck_return")?;
            self.result_pointer = Some(result);
            let zero = self.builder.const_int(self.types.i64, "0")?;
            let name = self.name("result.null");
            let null =
                self.builder
                    .cast(BridgeCastOp::IntToPtr, zero, self.types.pointer, &name)?;
            let name = self.name("result.is_null");
            let failed = self
                .builder
                .compare(BridgeCompareOp::IcmpEq, result, null, &name)?;
            self.guard_status(failed, self.status(3)?)?;
        }
        Ok(())
    }

    fn emit_contract_assumes(&mut self) -> Result<(), NativeError> {
        let assumptions = self
            .facts
            .contract_assumes
            .get(&self.function.id)
            .cloned()
            .unwrap_or_default();
        for assumption in assumptions {
            let left = self.assume_operand(&assumption.left, &assumption.type_node)?;
            let right = self.assume_operand(&assumption.right, &assumption.type_node)?;
            let name = self.name("contract.assume.condition");
            let condition = self.builder.compare(
                compare_op(assumption.op, &assumption.type_node),
                left,
                right,
                &name,
            )?;
            self.builder.assume(condition)?;
        }
        Ok(())
    }

    fn assume_operand(
        &mut self,
        operand: &AssumeOperand,
        type_node: &MirType,
    ) -> Result<NativeValue<'module>, NativeError> {
        match operand {
            AssumeOperand::Value(value) => self.load(*value),
            AssumeOperand::SliceLength(value) => {
                let slice = self.load(*value)?;
                let name = self.name("contract.slice.len");
                self.builder.extract_value(slice, 1, &name)
            }
            AssumeOperand::Constant(value) => {
                self.builder.const_int(self.types.get(type_node)?, value)
            }
        }
    }

    fn instruction(&mut self, instruction: &KirInstruction) -> Result<(), NativeError> {
        match &instruction.kind {
            KirInstructionKind::Undef { .. } => {
                let result = &instruction.results[0];
                let value = self.builder.undef(self.types.get(&result.type_node)?)?;
                self.store(result.value, value)
            }
            KirInstructionKind::ConstInt { value } => {
                let result = &instruction.results[0];
                let value = self
                    .builder
                    .const_int(self.types.get(&result.type_node)?, value)?;
                self.store(result.value, value)
            }
            KirInstructionKind::ConstFloat { value } => {
                let result = &instruction.results[0];
                let value = self
                    .builder
                    .const_float(self.types.get(&result.type_node)?, value)?;
                self.store(result.value, value)
            }
            KirInstructionKind::ConstBool { value } => {
                let result = instruction.results[0].value;
                let value = self.builder.const_bool(*value)?;
                self.store(result, value)
            }
            KirInstructionKind::Copy { value } => {
                let loaded = self.load(*value)?;
                self.store(instruction.results[0].value, loaded)
            }
            KirInstructionKind::Binary {
                op,
                left,
                right,
                semantics,
            } => self.binary(instruction, *op, *left, *right, *semantics),
            KirInstructionKind::Unary {
                op,
                operand,
                semantics,
            } => self.unary(instruction, *op, *operand, *semantics),
            KirInstructionKind::Compare { op, left, right } => {
                let left_value = self.load(*left)?;
                let right_value = self.load(*right)?;
                let name = self.name("compare");
                let value = self.builder.compare(
                    compare_op(*op, self.type_of(*left)?),
                    left_value,
                    right_value,
                    &name,
                )?;
                self.store(instruction.results[0].value, value)
            }
            KirInstructionKind::Cast { op, value } => {
                let value = self.load(*value)?;
                let op = match op {
                    MirCastOp::I32ToF64 => BridgeCastOp::Sitofp,
                    MirCastOp::U32ToF64 => BridgeCastOp::Uitofp,
                };
                let result = &instruction.results[0];
                let name = self.name("cast");
                let value =
                    self.builder
                        .cast(op, value, self.types.get(&result.type_node)?, &name)?;
                self.store(result.value, value)
            }
            KirInstructionKind::CheckCondition { kind, args } => {
                if !self
                    .guard_conditions
                    .contains(&instruction.results[0].value)
                {
                    return Ok(());
                }
                let value = self.check_condition(*kind, args)?;
                self.store(instruction.results[0].value, value)
            }
            KirInstructionKind::Guard { condition, failure } => {
                let failed = self.load(*condition)?;
                let code = match failure {
                    KirFailureKind::Overflow => 1,
                    KirFailureKind::DivisionByZero => 2,
                    KirFailureKind::OutOfBounds | KirFailureKind::ContractViolation => 4,
                };
                let status = self.status(code)?;
                self.guard_status(failed, status)
            }
            KirInstructionKind::Address { place } => {
                let pointer = self.place_pointer(place)?;
                self.store(instruction.results[0].value, pointer)
            }
            KirInstructionKind::Load { place } => {
                let pointer = self.place_pointer(place)?;
                let result = &instruction.results[0];
                let name = self.name("place.load");
                let (alias_scopes, noalias_scopes) = self.alias_metadata(place)?;
                let value = if alias_scopes.is_empty() && noalias_scopes.is_empty() {
                    self.builder
                        .load(self.types.get(&result.type_node)?, pointer, &name)?
                } else {
                    self.builder.load_scoped_alias(
                        self.types.get(&result.type_node)?,
                        pointer,
                        &alias_scopes,
                        &noalias_scopes,
                        &name,
                    )?
                };
                self.store(result.value, value)
            }
            KirInstructionKind::Store { place, value } => {
                let pointer = self.place_pointer(place)?;
                let value = self.load(*value)?;
                let (alias_scopes, noalias_scopes) = self.alias_metadata(place)?;
                if alias_scopes.is_empty() && noalias_scopes.is_empty() {
                    self.builder.store(value, pointer)
                } else {
                    self.builder
                        .store_scoped_alias(value, pointer, &alias_scopes, &noalias_scopes)
                }
            }
            KirInstructionKind::MakeSlice { data, len } => {
                let data = self.load(*data)?;
                let len = self.load(*len)?;
                let value = self.make_slice(data, len)?;
                self.store(instruction.results[0].value, value)
            }
            KirInstructionKind::SliceData { slice } => {
                let slice = self.load(*slice)?;
                let name = self.name("slice.data");
                let value = self.builder.extract_value(slice, 0, &name)?;
                self.store(instruction.results[0].value, value)
            }
            KirInstructionKind::SliceLen { slice } => {
                let slice = self.load(*slice)?;
                let name = self.name("slice.len");
                let value = self.builder.extract_value(slice, 1, &name)?;
                self.store(instruction.results[0].value, value)
            }
            KirInstructionKind::Subslice { slice, start, end } => {
                self.subslice(instruction, *slice, *start, *end)
            }
            KirInstructionKind::Call {
                function_name,
                args,
            } => self.call(instruction, function_name, args),
            KirInstructionKind::RuntimeCall { intrinsic, args } => {
                let (name, _) = runtime_signature(*intrinsic);
                let function = require_function(self.functions, name)?;
                let args = self.physical_args(args)?;
                self.builder.call(function, &args, "").map(|_| ())
            }
        }
    }

    fn binary(
        &mut self,
        instruction: &KirInstruction,
        op: MirBinaryOp,
        left: ValueId,
        right: ValueId,
        semantics: KirArithmeticSemantics,
    ) -> Result<(), NativeError> {
        let left_value = self.load(left)?;
        let right_value = self.load(right)?;
        let type_node = &instruction.results[0].type_node;
        if semantics == KirArithmeticSemantics::Checked
            && instruction.results.len() == 2
            && self
                .guard_conditions
                .contains(&instruction.results[1].value)
        {
            let unsigned = matches!(
                type_node,
                MirType::Primitive(MirPrimitiveTypeName::U32 | MirPrimitiveTypeName::U64)
            );
            let overflow_op = match (op, unsigned) {
                (MirBinaryOp::Add, false) => BridgeOverflowOp::SignedAdd,
                (MirBinaryOp::Add, true) => BridgeOverflowOp::UnsignedAdd,
                (MirBinaryOp::Sub, false) => BridgeOverflowOp::SignedSub,
                (MirBinaryOp::Sub, true) => BridgeOverflowOp::UnsignedSub,
                (MirBinaryOp::Mul, false) => BridgeOverflowOp::SignedMul,
                (MirBinaryOp::Mul, true) => BridgeOverflowOp::UnsignedMul,
                _ => return Err(lowering_error("invalid checked KIR binary pair")),
            };
            let name = self.name("overflow.pair");
            let pair = self
                .builder
                .overflow(overflow_op, left_value, right_value, &name)?;
            let name = self.name("overflow.value");
            let value = self.builder.extract_value(pair, 0, &name)?;
            let name = self.name("overflow.flag");
            let overflow = self.builder.extract_value(pair, 1, &name)?;
            self.store(instruction.results[0].value, value)?;
            return self.store(instruction.results[1].value, overflow);
        }
        let name = self.name("binary");
        let wrap = self
            .facts
            .wrap_proofs
            .get(&(self.function.id, instruction.id));
        let value = self.builder.binary_with_flags(
            binary_op(op, type_node)?,
            left_value,
            right_value,
            matches!(wrap, Some((_, NativeStrengtheningKind::NoUnsignedWrap))),
            matches!(wrap, Some((_, NativeStrengtheningKind::NoSignedWrap))),
            &name,
        )?;
        self.store(instruction.results[0].value, value)
    }

    fn unary(
        &mut self,
        instruction: &KirInstruction,
        op: MirUnaryOp,
        operand: ValueId,
        semantics: KirArithmeticSemantics,
    ) -> Result<(), NativeError> {
        let operand = self.load(operand)?;
        let type_node = &instruction.results[0].type_node;
        if semantics == KirArithmeticSemantics::Checked
            && instruction.results.len() == 2
            && self
                .guard_conditions
                .contains(&instruction.results[1].value)
        {
            let unsigned = matches!(
                type_node,
                MirType::Primitive(MirPrimitiveTypeName::U32 | MirPrimitiveTypeName::U64)
            );
            let op = if unsigned {
                BridgeOverflowOp::UnsignedSub
            } else {
                BridgeOverflowOp::SignedSub
            };
            let zero = self.builder.const_int(self.types.get(type_node)?, "0")?;
            let name = self.name("negate.pair");
            let pair = self.builder.overflow(op, zero, operand, &name)?;
            let name = self.name("negate.value");
            let value = self.builder.extract_value(pair, 0, &name)?;
            let name = self.name("negate.overflow");
            let overflow = self.builder.extract_value(pair, 1, &name)?;
            self.store(instruction.results[0].value, value)?;
            return self.store(instruction.results[1].value, overflow);
        }
        let name = self.name("unary");
        let value = self
            .builder
            .unary(unary_op(op, type_node), operand, &name)?;
        self.store(instruction.results[0].value, value)
    }

    fn check_condition(
        &mut self,
        kind: KirCheckConditionKind,
        args: &[ValueId],
    ) -> Result<NativeValue<'module>, NativeError> {
        match kind {
            KirCheckConditionKind::ArithmeticOverflow => self.builder.const_bool(false),
            KirCheckConditionKind::DivisionByZero => {
                let value = self.load(args[0])?;
                let zero = self
                    .builder
                    .const_int(self.types.get(self.type_of(args[0])?)?, "0")?;
                let name = self.name("division.by_zero");
                self.builder
                    .compare(BridgeCompareOp::IcmpEq, value, zero, &name)
            }
            KirCheckConditionKind::SignedDivisionOverflow => {
                let left = self.load(args[0])?;
                let right = self.load(args[1])?;
                let type_node = self.type_of(args[0])?;
                let minimum = match type_node {
                    MirType::Primitive(MirPrimitiveTypeName::I32) => "-2147483648",
                    MirType::Primitive(MirPrimitiveTypeName::I64) => "-9223372036854775808",
                    _ => return Err(lowering_error("signed division check type is invalid")),
                };
                let llvm_type = self.types.get(type_node)?;
                let minimum = self.builder.const_int(llvm_type, minimum)?;
                let negative_one = self.builder.const_int(llvm_type, "-1")?;
                let name = self.name("division.minimum");
                let is_min = self
                    .builder
                    .compare(BridgeCompareOp::IcmpEq, left, minimum, &name)?;
                let name = self.name("division.negative_one");
                let is_negative_one =
                    self.builder
                        .compare(BridgeCompareOp::IcmpEq, right, negative_one, &name)?;
                let false_value = self.builder.const_bool(false)?;
                let name = self.name("division.overflows");
                self.builder
                    .select(is_min, is_negative_one, false_value, &name)
            }
            KirCheckConditionKind::SliceOutOfBounds => {
                let slice = self.load(args[0])?;
                let index = self.load(args[1])?;
                let name = self.name("slice.len");
                let len = self.builder.extract_value(slice, 1, &name)?;
                let name = self.name("slice.out_of_bounds");
                self.builder
                    .compare(BridgeCompareOp::IcmpUge, index, len, &name)
            }
            KirCheckConditionKind::InvalidSubslice => {
                let slice = self.load(args[0])?;
                let start = self.load(args[1])?;
                let end = self.load(args[2])?;
                let name = self.name("subslice.len");
                let len = self.builder.extract_value(slice, 1, &name)?;
                let name = self.name("subslice.start_after_end");
                let invalid_order =
                    self.builder
                        .compare(BridgeCompareOp::IcmpUgt, start, end, &name)?;
                let name = self.name("subslice.end_after_len");
                let invalid_end =
                    self.builder
                        .compare(BridgeCompareOp::IcmpUgt, end, len, &name)?;
                let true_value = self.builder.const_bool(true)?;
                let name = self.name("subslice.invalid");
                self.builder
                    .select(invalid_order, true_value, invalid_end, &name)
            }
        }
    }

    fn subslice(
        &mut self,
        instruction: &KirInstruction,
        slice: ValueId,
        start: ValueId,
        end: ValueId,
    ) -> Result<(), NativeError> {
        let MirType::Slice(element) = self.type_of(slice)? else {
            return Err(lowering_error("KIR subslice source is not a slice"));
        };
        let element = element.clone();
        let descriptor = self.load(slice)?;
        let name = self.name("subslice.data");
        let data = self.builder.extract_value(descriptor, 0, &name)?;
        let start_value = self.load(start)?;
        let end_value = self.load(end)?;
        let start_type = self.type_of(start)?.clone();
        let start64 = self.index_to_i64(start_value, &start_type)?;
        let name = self.name("subslice.gep");
        let advanced = self
            .builder
            .gep(self.types.get(&element)?, data, &[start64], &name)?;
        let zero = self.builder.const_int(self.types.i32, "0")?;
        let name = self.name("subslice.zero");
        let is_zero = self
            .builder
            .compare(BridgeCompareOp::IcmpEq, start_value, zero, &name)?;
        let name = self.name("subslice.selected");
        let selected = self.builder.select(is_zero, data, advanced, &name)?;
        let name = self.name("subslice.length");
        let len = self.builder.binary(
            super::ffi::BridgeBinaryOp::Sub,
            end_value,
            start_value,
            &name,
        )?;
        let value = self.make_slice(selected, len)?;
        self.store(instruction.results[0].value, value)
    }

    fn call(
        &mut self,
        instruction: &KirInstruction,
        name: &str,
        args: &[ValueId],
    ) -> Result<(), NativeError> {
        let function = require_function(self.functions, name)?;
        let mut args = self.physical_args(args)?;
        if self.status_abi {
            if let Some(result) = instruction.results.first() {
                args.push(self.storage(result.value)?.pointer);
            }
            let call_name = self.name("call");
            let status = self.builder.call(function, &args, &call_name)?;
            let zero = self.status(0)?;
            let compare_name = self.name("call.failed");
            let failed =
                self.builder
                    .compare(BridgeCompareOp::IcmpNe, status, zero, &compare_name)?;
            self.guard_status(failed, status)
        } else if let Some(result) = instruction.results.first() {
            let call_name = self.name("call");
            let value = self.builder.call(function, &args, &call_name)?;
            self.store(result.value, value)
        } else {
            self.builder.call(function, &args, "").map(|_| ())
        }
    }

    fn terminator(
        &mut self,
        _source: BlockId,
        terminator: &KirTerminator,
    ) -> Result<(), NativeError> {
        match terminator {
            KirTerminator::Return { value, .. } => {
                if self.status_abi {
                    if let Some(value) = value {
                        let value = self.load(*value)?;
                        let pointer = self.result_pointer.ok_or_else(|| {
                            lowering_error("checked KIR return is missing result pointer")
                        })?;
                        self.builder.store(value, pointer)?;
                    }
                    let ok = self.status(0)?;
                    self.builder.return_value(ok)
                } else if let Some(value) = value {
                    let value = self.load(*value)?;
                    self.builder.return_value(value)
                } else {
                    self.builder.return_void()
                }
            }
            KirTerminator::Jump { edge } => {
                let source = self.current_block()?;
                let target = self.edge_block(edge)?;
                self.builder.position(source)?;
                self.builder.branch(target)
            }
            KirTerminator::Branch {
                condition,
                then_edge,
                else_edge,
            } => {
                let condition = self.load(*condition)?;
                let source = self.current_block()?;
                let then_block = self.edge_block(then_edge)?;
                self.builder.position(source)?;
                let else_block = self.edge_block(else_edge)?;
                self.builder.position(source)?;
                self.builder.cond_branch(condition, then_block, else_block)
            }
        }
    }

    fn edge_block(&mut self, edge: &KirEdge) -> Result<NativeBlock<'module>, NativeError> {
        let values = edge
            .args
            .iter()
            .map(|value| self.load(*value))
            .collect::<Result<Vec<_>, _>>()?;
        let name = self.name("kir.edge");
        let edge_block = self.handle.append_block(&name)?;
        self.builder.position(edge_block)?;
        let target = self
            .function
            .blocks
            .iter()
            .find(|block| block.id == edge.target)
            .ok_or_else(|| lowering_error("KIR edge target is missing"))?;
        for (param, value) in target.params.iter().zip(values) {
            self.store(param.value, value)?;
        }
        self.builder.branch(self.block(edge.target)?)?;
        Ok(edge_block)
    }

    fn current_block(&self) -> Result<NativeBlock<'module>, NativeError> {
        self.current_block
            .ok_or_else(|| lowering_error("KIR lowering has no active LLVM block"))
    }

    fn physical_args(
        &mut self,
        args: &[ValueId],
    ) -> Result<Vec<NativeValue<'module>>, NativeError> {
        let mut physical = Vec::new();
        for value in args {
            let loaded = self.load(*value)?;
            if matches!(self.type_of(*value)?, MirType::Slice(_)) {
                let name = self.name("arg.data");
                physical.push(self.builder.extract_value(loaded, 0, &name)?);
                let name = self.name("arg.len");
                physical.push(self.builder.extract_value(loaded, 1, &name)?);
            } else {
                physical.push(loaded);
            }
        }
        Ok(physical)
    }

    fn place_pointer(&mut self, place: &KirPlace) -> Result<NativeValue<'module>, NativeError> {
        match place {
            KirPlace::Value {
                value, type_node, ..
            } => {
                if matches!(type_node, MirType::Pointer(_)) {
                    self.load(*value)
                } else {
                    Ok(self.storage(*value)?.pointer)
                }
            }
            KirPlace::Deref { pointer, .. } => self.load(*pointer),
            KirPlace::Index { base, index, .. } => {
                let MirType::Pointer(element) = kir_place_type(base) else {
                    return Err(lowering_error("KIR index base is not a pointer"));
                };
                let element = element.clone();
                let base = self.place_pointer(base)?;
                let index_value = self.load(*index)?;
                let index_type = self.type_of(*index)?.clone();
                let index64 = self.index_to_i64(index_value, &index_type)?;
                let name = self.name("index");
                self.builder
                    .gep(self.types.get(&element)?, base, &[index64], &name)
            }
            KirPlace::SliceIndex { slice, index, .. } => {
                let MirType::Slice(element) = self.type_of(*slice)? else {
                    return Err(lowering_error("KIR slice index base is not a slice"));
                };
                let element = element.clone();
                let slice = self.load(*slice)?;
                let name = self.name("slice.data");
                let data = self.builder.extract_value(slice, 0, &name)?;
                let index_value = self.load(*index)?;
                let index_type = self.type_of(*index)?.clone();
                let index64 = self.index_to_i64(index_value, &index_type)?;
                let name = self.name("slice.index");
                self.builder
                    .gep(self.types.get(&element)?, data, &[index64], &name)
            }
            KirPlace::Field {
                base, field_name, ..
            } => {
                let MirType::Struct(struct_name) = kir_place_type(base) else {
                    return Err(lowering_error("KIR field base is not a struct"));
                };
                let struct_name = struct_name.clone();
                let base = self.place_pointer(base)?;
                let zero = self.builder.const_int(self.types.i32, "0")?;
                let field = self.builder.const_int(
                    self.types.i32,
                    &self
                        .layout
                        .field_index(&struct_name, field_name)
                        .to_string(),
                )?;
                let name = self.name("field");
                self.builder.gep(
                    self.types.get(&MirType::Struct(struct_name))?,
                    base,
                    &[zero, field],
                    &name,
                )
            }
        }
    }

    fn alias_metadata(&self, place: &KirPlace) -> Result<(Vec<u32>, Vec<u32>), NativeError> {
        let Some(root) = root_parameter_for_region(self.function, kir_place_region(place)) else {
            return Ok((Vec::new(), Vec::new()));
        };
        let facts = self
            .facts
            .scoped_alias_facts
            .get(&self.function.id)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let mut noalias = facts
            .iter()
            .filter_map(|(_, left, right)| {
                if *left == root {
                    Some(*right)
                } else if *right == root {
                    Some(*left)
                } else {
                    None
                }
            })
            .map(alias_scope_id)
            .collect::<Result<Vec<_>, _>>()?;
        noalias.sort_unstable();
        noalias.dedup();
        if noalias.is_empty() {
            return Ok((Vec::new(), Vec::new()));
        }
        Ok((vec![alias_scope_id(root)?], noalias))
    }

    fn index_to_i64(
        &mut self,
        value: NativeValue<'module>,
        type_node: &MirType,
    ) -> Result<NativeValue<'module>, NativeError> {
        match type_node {
            MirType::Primitive(MirPrimitiveTypeName::I32) => {
                let name = self.name("index64");
                self.builder
                    .cast(BridgeCastOp::Sext, value, self.types.i64, &name)
            }
            MirType::Primitive(MirPrimitiveTypeName::U32) => {
                let name = self.name("index64");
                self.builder
                    .cast(BridgeCastOp::Zext, value, self.types.i64, &name)
            }
            MirType::Primitive(MirPrimitiveTypeName::I64 | MirPrimitiveTypeName::U64) => Ok(value),
            _ => Err(lowering_error("KIR index is not an integer")),
        }
    }

    fn make_slice(
        &mut self,
        data: NativeValue<'module>,
        len: NativeValue<'module>,
    ) -> Result<NativeValue<'module>, NativeError> {
        let undef = self.builder.undef(self.types.slice)?;
        let name = self.name("slice.data");
        let with_data = self.builder.insert_value(undef, data, 0, &name)?;
        let name = self.name("slice.value");
        self.builder.insert_value(with_data, len, 1, &name)
    }

    fn guard_status(
        &mut self,
        failed: NativeValue<'module>,
        status: NativeValue<'module>,
    ) -> Result<(), NativeError> {
        let failure_name = self.name("checked.failure");
        let continue_name = self.name("checked.continue");
        let failure = self.handle.append_block(&failure_name)?;
        let continuation = self.handle.append_block(&continue_name)?;
        self.builder.cond_branch(failed, failure, continuation)?;
        self.builder.position(failure)?;
        self.builder.return_value(status)?;
        self.builder.position(continuation)?;
        self.current_block = Some(continuation);
        Ok(())
    }

    fn status(&self, code: i32) -> Result<NativeValue<'module>, NativeError> {
        self.builder.const_int(self.types.i32, &code.to_string())
    }

    fn load(&mut self, value: ValueId) -> Result<NativeValue<'module>, NativeError> {
        let storage = self.storage(value)?;
        let type_node = self.type_of(value)?.clone();
        let name = self.name("load");
        self.builder
            .load(self.types.get(&type_node)?, storage.pointer, &name)
    }

    fn store(&mut self, value: ValueId, native: NativeValue<'module>) -> Result<(), NativeError> {
        self.builder.store(native, self.storage(value)?.pointer)
    }

    fn storage(&self, value: ValueId) -> Result<Storage<'module>, NativeError> {
        self.storage
            .get(&value)
            .copied()
            .ok_or_else(|| lowering_error(format!("missing KIR storage for v{}", value.index())))
    }

    fn type_of(&self, value: ValueId) -> Result<&MirType, NativeError> {
        self.function
            .params
            .iter()
            .find(|param| param.value == value)
            .map(|param| &param.type_node)
            .or_else(|| {
                self.function
                    .blocks
                    .iter()
                    .flat_map(|block| &block.params)
                    .find(|param| param.value == value)
                    .map(|param| &param.type_node)
            })
            .or_else(|| {
                self.function
                    .blocks
                    .iter()
                    .flat_map(|block| &block.instructions)
                    .flat_map(|instruction| &instruction.results)
                    .find(|result| result.value == value)
                    .map(|result| &result.type_node)
            })
            .ok_or_else(|| lowering_error(format!("unknown KIR value v{}", value.index())))
    }
}

fn value_types(function: &KirFunction) -> BTreeMap<ValueId, MirType> {
    function
        .params
        .iter()
        .map(|param| (param.value, param.type_node.clone()))
        .chain(function.blocks.iter().flat_map(|block| {
            block
                .params
                .iter()
                .map(|param| (param.value, param.type_node.clone()))
                .chain(block.instructions.iter().flat_map(|instruction| {
                    instruction
                        .results
                        .iter()
                        .map(|result| (result.value, result.type_node.clone()))
                }))
        }))
        .collect()
}

fn kir_place_type(place: &KirPlace) -> &MirType {
    match place {
        KirPlace::Value { type_node, .. }
        | KirPlace::Deref { type_node, .. }
        | KirPlace::Index { type_node, .. }
        | KirPlace::SliceIndex { type_node, .. }
        | KirPlace::Field { type_node, .. } => type_node,
    }
}

fn kir_place_region(place: &KirPlace) -> MemoryRegionId {
    match place {
        KirPlace::Value { region, .. }
        | KirPlace::Deref { region, .. }
        | KirPlace::Index { region, .. }
        | KirPlace::SliceIndex { region, .. }
        | KirPlace::Field { region, .. } => *region,
    }
}

fn root_parameter_for_region(
    function: &KirFunction,
    mut region: MemoryRegionId,
) -> Option<ValueId> {
    let mut visited = HashSet::new();
    while visited.insert(region) {
        let descriptor = function
            .regions
            .iter()
            .find(|candidate| candidate.id == region)?;
        match descriptor.origin {
            KirMemoryRegionOrigin::Parameter(value) | KirMemoryRegionOrigin::RawSlice(value)
                if function.params.iter().any(|param| param.value == value) =>
            {
                return Some(value);
            }
            _ => {}
        }
        region = descriptor.parent?;
    }
    None
}

fn alias_scope_id(value: ValueId) -> Result<u32, NativeError> {
    value
        .index()
        .checked_add(1)
        .ok_or_else(|| lowering_error("KIR alias scope identity overflow"))
}

fn require_function<'module>(
    functions: &HashMap<String, NativeFunction<'module>>,
    name: &str,
) -> Result<NativeFunction<'module>, NativeError> {
    functions
        .get(name)
        .copied()
        .ok_or_else(|| lowering_error(format!("unknown KIR function '{name}'")))
}
