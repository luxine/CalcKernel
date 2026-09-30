use num_bigint::BigInt;

use crate::{
    CandidateKey, CanonicalLoopDescriptor, FunctionId, KirAlignmentClass, KirArithmeticSemantics,
    KirCostEstimate, KirCostKey, KirCostSemantics, KirInstruction, KirInstructionKind, KirLaneType,
    KirOperationAvailability, KirPlace, KirProfileOperation, KirTerminator,
    KirVerifiedProgramState, LoopCandidateKind, LoopCandidateVariant, LoopId, LoopTripCount,
    MemoryRegionId, MirBinaryOp, MirCompareOp, MirPrimitiveTypeName, MirType, ValueId,
};

use super::{AliasKind, IntegerType, analyze_regions, query_alias};

const RUNTIME_UNROLL_FACTOR: u8 = 4;
const MINIMUM_REDUCTION_PERCENT: u32 = 10;
const MAX_COST_SEARCH_TRIP: u32 = 4096;
const WASM_BRANCH_COST: u32 = 1;
const WASM_SLICE_LEN_COST: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeScalarUnrollKind {
    U32ModularSum,
    StrictF64DirectMap,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeScalarUnrollCandidate {
    pub key: CandidateKey,
    pub function: FunctionId,
    pub loop_id: LoopId,
    pub preheader: crate::BlockId,
    pub header: crate::BlockId,
    pub body: crate::BlockId,
    pub exit: crate::BlockId,
    pub induction: ValueId,
    pub bound: ValueId,
    pub bound_slice: ValueId,
    pub induction_update: crate::InstructionId,
    pub kind: RuntimeScalarUnrollKind,
    pub factor: u8,
    pub minimum_trip: u32,
    pub input_slice: ValueId,
    pub output_slice: Option<ValueId>,
    pub input_region: MemoryRegionId,
    pub output_region: Option<MemoryRegionId>,
    pub noalias_fact: Option<crate::FactId>,
    pub accumulator: Option<ValueId>,
    pub body_instructions: Vec<crate::InstructionId>,
    pub predicted_cost: KirCostEstimate,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RuntimeScalarUnrollDiscovery {
    pub candidates: Vec<RuntimeScalarUnrollCandidate>,
    pub fallbacks: Vec<RuntimeScalarUnrollFallback>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeScalarUnrollFallback {
    pub function: FunctionId,
    pub loop_id: LoopId,
    pub reason: String,
}

#[derive(Clone, Copy)]
struct SourceShape<'a> {
    descriptor: &'a CanonicalLoopDescriptor,
    preheader: &'a crate::KirBlock,
    header: &'a crate::KirBlock,
    body: &'a crate::KirBlock,
    exit: &'a crate::KirBlock,
    incoming: &'a crate::KirEdge,
    then_edge: &'a crate::KirEdge,
    backedge: &'a crate::KirEdge,
    induction_index: usize,
    bound_slice: ValueId,
}

#[derive(Clone, Copy)]
struct RecognizedBody {
    kind: RuntimeScalarUnrollKind,
    input_slice: ValueId,
    input_region: MemoryRegionId,
    output_slice: Option<ValueId>,
    output_region: Option<MemoryRegionId>,
    accumulator: Option<ValueId>,
    induction_update: crate::InstructionId,
}

#[must_use]
pub fn discover_wasm_runtime_scalar_unroll_candidates(
    state: &KirVerifiedProgramState,
    descriptors: &[CanonicalLoopDescriptor],
) -> RuntimeScalarUnrollDiscovery {
    let mut result = RuntimeScalarUnrollDiscovery::default();
    let module = state.module();
    if module.config.consumer != crate::KirConsumer::WebAssembly
        || module.profile.wasm_features() != Some(crate::KirWasmFeatures::Baseline)
        || module.config.overflow_mode != crate::KirOverflowMode::Unchecked
        || module.config.bounds_mode != crate::KirBoundsMode::Unchecked
        || module.config.sanitizer_mode != crate::KirSanitizerMode::Disabled
    {
        return result;
    }
    for descriptor in descriptors.iter().filter(|descriptor| descriptor.innermost) {
        match discover_one(state, descriptor) {
            Some(candidate) => result.candidates.push(candidate),
            None => result.fallbacks.push(RuntimeScalarUnrollFallback {
                function: descriptor.function,
                loop_id: descriptor.id,
                reason: "runtime-scalar-unroll-source-shape-or-profitability".to_string(),
            }),
        }
    }
    result
        .candidates
        .sort_by(|left, right| left.key.cmp(&right.key));
    result.fallbacks.sort_by(|left, right| {
        (left.function, left.loop_id, &left.reason).cmp(&(
            right.function,
            right.loop_id,
            &right.reason,
        ))
    });
    result
}

fn discover_one(
    state: &KirVerifiedProgramState,
    descriptor: &CanonicalLoopDescriptor,
) -> Option<RuntimeScalarUnrollCandidate> {
    let function = state
        .module()
        .functions
        .iter()
        .find(|function| function.id == descriptor.function)?;
    let shape = source_shape(function, descriptor)?;
    let induction = descriptor.induction.as_ref()?;
    if induction.type_node != IntegerType::U32
        || induction.start != BigInt::from(0_u8)
        || induction.step != BigInt::from(1_u8)
        || induction.comparison != MirCompareOp::Lt
        || !induction.wrap_safe_for_strict_bound
        || !matches!(descriptor.trip_count, LoopTripCount::Runtime { .. })
        || induction.value != shape.header.params[shape.induction_index].value
    {
        return None;
    }
    let recognized = recognize_body(function, &shape)?;
    let written_memory = recognized.output_region.and_then(|region| {
        shape.body.instructions.iter().find_map(|instruction| {
            let KirInstructionKind::Store { place, .. } = &instruction.kind else {
                return None;
            };
            let KirPlace::SliceIndex {
                region: store_region,
                ..
            } = place.as_ref()
            else {
                return None;
            };
            if *store_region != region {
                return None;
            }
            instruction
                .memory
                .as_ref()
                .and_then(|memory| memory.output.map(|version| (memory.region, version)))
        })
    });
    if !memory_backedge_is_closed(
        &shape,
        written_memory.map(|(region, _)| region),
        written_memory.map(|(_, version)| version),
    ) {
        return None;
    }
    let noalias_fact = match recognized.output_region {
        Some(output_region) => {
            let regions = analyze_regions(
                function,
                state.contract_facts().map(crate::ContractFactSet::facts),
            )
            .ok()?;
            let alias = query_alias(&regions, output_region, recognized.input_region);
            if alias.kind != AliasKind::NoAlias {
                return None;
            }
            let fact = alias.fact?;
            Some(fact)
        }
        None => None,
    };
    let (minimum_trip, predicted_cost) = profitable_threshold(
        &state.module().profile,
        shape.body,
        function,
        RUNTIME_UNROLL_FACTOR,
    )?;
    let body_instructions = shape
        .body
        .instructions
        .iter()
        .map(|instruction| instruction.id)
        .collect::<Vec<_>>();
    Some(RuntimeScalarUnrollCandidate {
        key: CandidateKey::LoopFrontier {
            function: function.id,
            loop_id: descriptor.id,
            kind: LoopCandidateKind::RuntimeScalarUnroll,
            variant: LoopCandidateVariant::Scalar,
            vf: 1,
            uf: RUNTIME_UNROLL_FACTOR,
        },
        function: function.id,
        loop_id: descriptor.id,
        preheader: shape.preheader.id,
        header: shape.header.id,
        body: shape.body.id,
        exit: shape.exit.id,
        induction: induction.value,
        bound: induction.bound,
        bound_slice: shape.bound_slice,
        induction_update: recognized.induction_update,
        kind: recognized.kind,
        factor: RUNTIME_UNROLL_FACTOR,
        minimum_trip,
        input_slice: recognized.input_slice,
        output_slice: recognized.output_slice,
        input_region: recognized.input_region,
        output_region: recognized.output_region,
        noalias_fact,
        accumulator: recognized.accumulator,
        body_instructions,
        predicted_cost,
    })
}

fn source_shape<'a>(
    function: &'a crate::KirFunction,
    descriptor: &'a CanonicalLoopDescriptor,
) -> Option<SourceShape<'a>> {
    if !descriptor.innermost
        || !descriptor.dedicated_exits
        || !descriptor.lcssa
        || descriptor.blocks.len() != 2
        || descriptor.exits.len() != 1
        || !matches!(descriptor.trip_count, LoopTripCount::Runtime { .. })
    {
        return None;
    }
    let induction = descriptor.induction.as_ref()?;
    let preheader = block(function, descriptor.preheader?)?;
    let header = block(function, descriptor.header)?;
    let body = block(function, descriptor.latch?)?;
    let exit = block(function, descriptor.exits[0])?;
    if body.id == header.id || !descriptor.blocks.contains(&body.id) {
        return None;
    }
    let KirTerminator::Jump { edge: incoming } = &preheader.terminator else {
        return None;
    };
    let KirTerminator::Branch {
        condition,
        then_edge,
        else_edge,
    } = &header.terminator
    else {
        return None;
    };
    let KirTerminator::Jump { edge: backedge } = &body.terminator else {
        return None;
    };
    if incoming.target != header.id
        || then_edge.target != body.id
        || else_edge.target != exit.id
        || backedge.target != header.id
        || incoming.args.len() != header.params.len()
        || then_edge.args.len() != body.params.len()
        || backedge.args.len() != header.params.len()
        || incoming.memory_args.len() != header.memory_params.len()
        || then_edge.memory_args.len() != body.memory_params.len()
        || backedge.memory_args.len() != header.memory_params.len()
        || then_edge.args.iter().any(|argument| {
            !header
                .params
                .iter()
                .any(|parameter| parameter.value == *argument)
        })
    {
        return None;
    }
    let induction_index = header
        .params
        .iter()
        .position(|parameter| parameter.value == induction.value)?;
    let bounds = header
        .instructions
        .iter()
        .filter(|instruction| {
            instruction
                .results
                .iter()
                .any(|result| result.value == induction.bound)
        })
        .collect::<Vec<_>>();
    let [bound_instruction] = bounds.as_slice() else {
        return None;
    };
    let KirInstructionKind::SliceLen { slice: bound_slice } = &bound_instruction.kind else {
        return None;
    };
    let bound_slice = *bound_slice;
    if bound_instruction.memory.is_some() || bound_instruction.effect.is_some() {
        return None;
    }
    let bound_slice_index = header
        .params
        .iter()
        .position(|parameter| parameter.value == bound_slice)?;
    let bound_body_index = then_edge
        .args
        .iter()
        .position(|argument| *argument == bound_slice)?;
    let bound_body_param = body.params.get(bound_body_index)?;
    let condition_definitions = header
        .instructions
        .iter()
        .filter(|instruction| {
            instruction
                .results
                .iter()
                .any(|result| result.value == *condition)
        })
        .collect::<Vec<_>>();
    let [condition_instruction] = condition_definitions.as_slice() else {
        return None;
    };
    if !matches!(&condition_instruction.kind, KirInstructionKind::Compare {
        op: MirCompareOp::Lt,
        left,
        right,
    } if *left == induction.value && *right == induction.bound)
        || condition_instruction.memory.is_some()
        || condition_instruction.effect.is_some()
        || header.instructions.iter().any(|instruction| {
            instruction.id != bound_instruction.id && instruction.id != condition_instruction.id
        })
    {
        return None;
    }
    // The bound slice must be an unchanged source header parameter. Its value
    // at entry is therefore available before the first loop test.
    let incoming_slice = incoming.args.get(bound_slice_index).copied()?;
    if backedge.args.get(bound_slice_index) != Some(&bound_body_param.value) {
        return None;
    }
    if incoming_slice == induction.value {
        return None;
    }
    Some(SourceShape {
        descriptor,
        preheader,
        header,
        body,
        exit,
        incoming,
        then_edge,
        backedge,
        induction_index,
        bound_slice,
    })
}

fn recognize_body(
    function: &crate::KirFunction,
    shape: &SourceShape<'_>,
) -> Option<RecognizedBody> {
    let induction = shape.descriptor.induction.as_ref()?;
    let body_induction_index = shape
        .then_edge
        .args
        .iter()
        .position(|argument| *argument == induction.value)?;
    let body_induction = shape.body.params.get(body_induction_index)?.value;
    let induction_update = shape.body.instructions.last()?;
    let [induction_result] = induction_update.results.as_slice() else {
        return None;
    };
    let step_value = operand_right(&induction_update.kind)?;
    let step_one = constant_u32(function, step_value)?;
    if step_one != 1
        || !matches!(&induction_update.kind, KirInstructionKind::Binary {
            op: MirBinaryOp::Add,
            left,
            right: _,
            semantics: KirArithmeticSemantics::Modular,
        } if *left == body_induction)
        || !type_is(&induction_result.type_node, MirPrimitiveTypeName::U32)
        || shape.backedge.args.get(shape.induction_index) != Some(&induction_result.value)
    {
        return None;
    }
    let body_without_update = &shape.body.instructions[..shape.body.instructions.len() - 1];
    let body_semantics = body_without_update
        .iter()
        .filter(|instruction| {
            !instruction
                .results
                .iter()
                .any(|result| result.value == step_value)
        })
        .cloned()
        .collect::<Vec<_>>();
    let step_definitions = body_without_update
        .iter()
        .filter(|instruction| {
            instruction
                .results
                .iter()
                .any(|result| result.value == step_value)
        })
        .collect::<Vec<_>>();
    if let Some(step_definition) = step_definitions.first()
        && (step_definitions.len() != 1
            || !matches!(&step_definition.kind, KirInstructionKind::ConstInt { value } if value == "1")
            || !type_is(
                &step_definition.results.first()?.type_node,
                MirPrimitiveTypeName::U32,
            ))
    {
        return None;
    }
    if step_definitions.len() > 1 {
        return None;
    }
    let body_semantics = body_semantics.iter().collect::<Vec<_>>();
    if let Some(recognized) = recognize_sum(shape, &body_semantics, induction_update.id) {
        return Some(recognized);
    }
    recognize_map(function, shape, &body_semantics, induction_update.id)
}

fn recognize_sum(
    shape: &SourceShape<'_>,
    body: &[&KirInstruction],
    induction_update: crate::InstructionId,
) -> Option<RecognizedBody> {
    let [load, add] = body else {
        return None;
    };
    let KirInstructionKind::Load { place } = &load.kind else {
        return None;
    };
    let KirPlace::SliceIndex {
        slice,
        index,
        type_node: MirType::Primitive(MirPrimitiveTypeName::U32),
        region,
    } = place.as_ref()
    else {
        return None;
    };
    let load_memory = load.memory.as_ref()?;
    if load_memory.output.is_some()
        || load.effect.as_ref()?.kind != crate::KirEffectKind::ReadMemory
        || load.results.len() != 1
    {
        return None;
    }
    let induction_value = shape.descriptor.induction.as_ref()?.value;
    let body_induction_index = shape
        .then_edge
        .args
        .iter()
        .position(|argument| *argument == induction_value)?;
    let body_induction = shape.body.params.get(body_induction_index)?.value;
    if *index != body_induction || !body_slice_matches_header(shape, *slice, shape.bound_slice) {
        return None;
    }
    if shape
        .body
        .memory_params
        .iter()
        .find(|parameter| parameter.region == load_memory.region)
        .is_none_or(|parameter| parameter.version != load_memory.input)
    {
        return None;
    }
    let [load_result] = load.results.as_slice() else {
        return None;
    };
    let KirInstructionKind::Binary {
        op: MirBinaryOp::Add,
        left,
        right,
        semantics: KirArithmeticSemantics::Modular,
    } = &add.kind
    else {
        return None;
    };
    let body_accumulator_index = shape
        .body
        .params
        .iter()
        .position(|parameter| parameter.value == *left)?;
    let header_accumulator = *shape.then_edge.args.get(body_accumulator_index)?;
    if *right != load_result.value
        || add.results.len() != 1
        || !type_is(&add.results[0].type_node, MirPrimitiveTypeName::U32)
        || shape
            .header
            .params
            .iter()
            .zip(&shape.backedge.args)
            .find(|(parameter, _)| parameter.value == header_accumulator)
            .is_none_or(|(_, back_value)| *back_value != add.results[0].value)
        || !backedge_forwards_all_but(shape, &[header_accumulator], &[add.results[0].value])
        || body_has_unexpected_memory_effect(shape, &[load.id])
    {
        return None;
    }
    let input_header_value = body_param_header_value(shape, *slice)?;
    Some(RecognizedBody {
        kind: RuntimeScalarUnrollKind::U32ModularSum,
        input_slice: input_header_value,
        input_region: *region,
        output_slice: None,
        output_region: None,
        accumulator: Some(header_accumulator),
        induction_update,
    })
}

fn recognize_map(
    function: &crate::KirFunction,
    shape: &SourceShape<'_>,
    body: &[&KirInstruction],
    induction_update: crate::InstructionId,
) -> Option<RecognizedBody> {
    let [load, mul_const, mul, add_const, add, store] = body else {
        return None;
    };
    let KirInstructionKind::Load { place } = &load.kind else {
        return None;
    };
    let KirPlace::SliceIndex {
        slice: input_body,
        index: load_index,
        type_node: MirType::Primitive(MirPrimitiveTypeName::F64),
        region: input_region,
    } = place.as_ref()
    else {
        return None;
    };
    let KirInstructionKind::ConstFloat { .. } = &mul_const.kind else {
        return None;
    };
    let [loaded] = load.results.as_slice() else {
        return None;
    };
    let [multiplier] = mul_const.results.as_slice() else {
        return None;
    };
    let KirInstructionKind::Binary {
        op: MirBinaryOp::Mul,
        left,
        right,
        semantics: KirArithmeticSemantics::StrictFloat,
    } = &mul.kind
    else {
        return None;
    };
    if *left != loaded.value || *right != multiplier.value {
        return None;
    }
    let KirInstructionKind::ConstFloat { .. } = &add_const.kind else {
        return None;
    };
    let [product] = mul.results.as_slice() else {
        return None;
    };
    let [addend] = add_const.results.as_slice() else {
        return None;
    };
    let KirInstructionKind::Binary {
        op: MirBinaryOp::Add,
        left: product_value,
        right: addend_value,
        semantics: KirArithmeticSemantics::StrictFloat,
    } = &add.kind
    else {
        return None;
    };
    if *product_value != product.value || *addend_value != addend.value {
        return None;
    }
    let KirInstructionKind::Store { place, value } = &store.kind else {
        return None;
    };
    let KirPlace::SliceIndex {
        slice: output_body,
        index: store_index,
        type_node: MirType::Primitive(MirPrimitiveTypeName::F64),
        region: output_region,
    } = place.as_ref()
    else {
        return None;
    };
    let load_memory = load.memory.as_ref()?;
    let store_memory = store.memory.as_ref()?;
    if load.effect.as_ref()?.kind != crate::KirEffectKind::ReadMemory
        || load_memory.output.is_some()
        || store.effect.as_ref()?.kind != crate::KirEffectKind::WriteMemory
        || store_memory.output.is_none()
        || shape
            .body
            .memory_params
            .iter()
            .find(|parameter| parameter.region == load_memory.region)
            .is_none_or(|parameter| parameter.version != load_memory.input)
        || shape
            .body
            .memory_params
            .iter()
            .find(|parameter| parameter.region == store_memory.region)
            .is_none_or(|parameter| parameter.version != store_memory.input)
        || load.effect.as_ref()?.order >= store.effect.as_ref()?.order
        || *value != add.results.first()?.value
        || !store.results.is_empty()
        || *load_index != *store_index
        || shape
            .body
            .params
            .get(shape.induction_index_for_body()?)?
            .value
            != *load_index
        || body_param_header_value(shape, *input_body)? != shape.bound_slice
        || body_param_header_value(shape, *output_body)? == shape.bound_slice
        || !backedge_forwards_all_but(shape, &[], &[])
        || body_has_unexpected_memory_effect(shape, &[load.id, store.id])
    {
        return None;
    }
    let input_slice = body_param_header_value(shape, *input_body)?;
    let output_slice = body_param_header_value(shape, *output_body)?;
    let all_body_ids = [load, mul_const, mul, add_const, add, store];
    if all_body_ids
        .iter()
        .any(|instruction| instruction.memory.is_some() != instruction.effect.is_some())
        || function.id != shape.descriptor.function
    {
        return None;
    }
    Some(RecognizedBody {
        kind: RuntimeScalarUnrollKind::StrictF64DirectMap,
        input_slice,
        input_region: *input_region,
        output_slice: Some(output_slice),
        output_region: Some(*output_region),
        accumulator: None,
        induction_update,
    })
}

impl SourceShape<'_> {
    fn induction_index_for_body(&self) -> Option<usize> {
        let induction_value = self.descriptor.induction.as_ref()?.value;
        self.then_edge
            .args
            .iter()
            .position(|argument| *argument == induction_value)
    }
}

fn body_param_header_value(shape: &SourceShape<'_>, body_value: ValueId) -> Option<ValueId> {
    let body_index = shape
        .body
        .params
        .iter()
        .position(|parameter| parameter.value == body_value)?;
    shape.then_edge.args.get(body_index).copied()
}

fn body_slice_matches_header(
    shape: &SourceShape<'_>,
    body_slice: ValueId,
    expected: ValueId,
) -> bool {
    let Some(body_index) = shape
        .body
        .params
        .iter()
        .position(|parameter| parameter.value == body_slice)
    else {
        return false;
    };
    let Some(header_index) = shape
        .header
        .params
        .iter()
        .position(|parameter| parameter.value == expected)
    else {
        return false;
    };
    shape.then_edge.args.get(body_index) == Some(&expected)
        && shape.backedge.args.get(header_index) == Some(&body_slice)
        && shape.incoming.args.get(header_index).is_some()
}

fn backedge_forwards_all_but(
    shape: &SourceShape<'_>,
    updated_header_values: &[ValueId],
    updated_body_values: &[ValueId],
) -> bool {
    if updated_header_values.len() != updated_body_values.len() {
        return false;
    }
    let induction = shape.descriptor.induction.as_ref().map(|item| item.value);
    shape
        .header
        .params
        .iter()
        .enumerate()
        .all(|(header_index, parameter)| {
            let Some(back_value) = shape.backedge.args.get(header_index) else {
                return false;
            };
            if Some(parameter.value) == induction {
                return shape
                    .body
                    .instructions
                    .iter()
                    .flat_map(|instruction| &instruction.results)
                    .any(|result| result.value == *back_value);
            }
            if let Some(updated_index) = updated_header_values
                .iter()
                .position(|value| *value == parameter.value)
            {
                return updated_body_values.get(updated_index) == Some(back_value);
            }
            let Some(body_index) = shape
                .then_edge
                .args
                .iter()
                .position(|value| *value == parameter.value)
            else {
                return false;
            };
            shape
                .body
                .params
                .get(body_index)
                .is_some_and(|body_parameter| body_parameter.value == *back_value)
        })
}

fn memory_backedge_is_closed(
    shape: &SourceShape<'_>,
    written_region: Option<MemoryRegionId>,
    written_version: Option<crate::MemoryVersionId>,
) -> bool {
    if written_region.is_some() != written_version.is_some()
        || shape.header.memory_params.len() != shape.incoming.memory_args.len()
        || shape.header.memory_params.len() != shape.backedge.memory_args.len()
        || shape.body.memory_params.len() != shape.then_edge.memory_args.len()
    {
        return false;
    }
    shape
        .header
        .memory_params
        .iter()
        .enumerate()
        .all(|(header_index, header_param)| {
            let Some(body_version) = shape.then_edge.memory_args.get(header_index) else {
                return false;
            };
            let Some(body_param) = shape.body.memory_params.get(header_index) else {
                return false;
            };
            if *body_version != header_param.version || body_param.region != header_param.region {
                return false;
            }
            let expected = if Some(header_param.region) == written_region {
                written_version
            } else {
                Some(body_param.version)
            };
            shape.backedge.memory_args.get(header_index).copied() == expected
        })
}

fn body_has_unexpected_memory_effect(
    shape: &SourceShape<'_>,
    allowed: &[crate::InstructionId],
) -> bool {
    let allowed = allowed
        .iter()
        .copied()
        .collect::<std::collections::BTreeSet<_>>();
    shape.body.instructions.iter().any(|instruction| {
        (instruction.memory.is_some() || instruction.effect.is_some())
            && !allowed.contains(&instruction.id)
    }) || shape
        .body
        .instructions
        .iter()
        .filter_map(|instruction| instruction.effect.as_ref())
        .any(|effect| {
            !matches!(
                effect.kind,
                crate::KirEffectKind::ReadMemory | crate::KirEffectKind::WriteMemory
            )
        })
}

fn profitable_threshold(
    profile: &crate::KirTargetProfile,
    body: &crate::KirBlock,
    function: &crate::KirFunction,
    factor: u8,
) -> Option<(u32, KirCostEstimate)> {
    let body_cost = body
        .instructions
        .iter()
        .try_fold(0_u32, |sum, instruction| {
            sum.checked_add(instruction_cost(profile, instruction, function)?)
        })?;
    let induction_type = KirLaneType::U32;
    let control = operation_cost(
        profile,
        KirProfileOperation::Compare,
        induction_type,
        KirCostSemantics::NotApplicable,
    )?
    .checked_add(WASM_BRANCH_COST)?;
    let source_control = control.checked_add(WASM_SLICE_LEN_COST)?;
    let remainder_cost = operation_cost(
        profile,
        KirProfileOperation::Remainder,
        induction_type,
        KirCostSemantics::Modular,
    )?;
    let subtract_cost = operation_cost(
        profile,
        KirProfileOperation::Subtract,
        induction_type,
        KirCostSemantics::Modular,
    )?;
    let guard_setup = WASM_SLICE_LEN_COST
        .checked_add(control)?
        .checked_add(remainder_cost)?
        .checked_add(subtract_cost)?;
    if body_cost == 0 {
        return None;
    }
    let factor_u32 = u32::from(factor);
    for trip in factor_u32..=MAX_COST_SEARCH_TRIP {
        let groups = trip / factor_u32;
        let tail = trip % factor_u32;
        let scalar = body_cost
            .checked_mul(trip)?
            .checked_add(source_control.checked_mul(trip.checked_add(1)?)?)?;
        let transformed_body = body_cost
            .checked_mul(groups.checked_mul(factor_u32)?)?
            .checked_add(control.checked_mul(groups.checked_add(1)?)?)?;
        let epilogue = body_cost
            .checked_mul(tail)?
            .checked_add(source_control.checked_mul(tail.checked_add(1)?)?)?;
        let total = transformed_body
            .checked_add(epilogue)?
            .checked_add(guard_setup)?;
        if u64::from(total).saturating_mul(100)
            <= u64::from(scalar).saturating_mul(u64::from(100 - MINIMUM_REDUCTION_PERCENT))
        {
            return Some((
                trip,
                KirCostEstimate::new(scalar, transformed_body, guard_setup, epilogue),
            ));
        }
    }
    None
}

fn instruction_cost(
    profile: &crate::KirTargetProfile,
    instruction: &KirInstruction,
    function: &crate::KirFunction,
) -> Option<u32> {
    let (operation, lane, semantics, alignment) = match &instruction.kind {
        KirInstructionKind::ConstInt { .. }
        | KirInstructionKind::ConstFloat { .. }
        | KirInstructionKind::ConstBool { .. }
        | KirInstructionKind::Copy { .. } => return Some(0),
        KirInstructionKind::Load { place } => (
            KirProfileOperation::Load,
            lane_for_type(place_type(place.as_ref())?)?,
            KirCostSemantics::NotApplicable,
            KirAlignmentClass::Bytes(byte_width(place_type(place.as_ref())?)?),
        ),
        KirInstructionKind::Store { place, .. } => (
            KirProfileOperation::Store,
            lane_for_type(place_type(place.as_ref())?)?,
            KirCostSemantics::NotApplicable,
            KirAlignmentClass::Bytes(byte_width(place_type(place.as_ref())?)?),
        ),
        KirInstructionKind::Binary { op, semantics, .. } => (
            match op {
                MirBinaryOp::Add => KirProfileOperation::Add,
                MirBinaryOp::Sub => KirProfileOperation::Subtract,
                MirBinaryOp::Mul => KirProfileOperation::Multiply,
                MirBinaryOp::Div => KirProfileOperation::Divide,
                MirBinaryOp::Mod => KirProfileOperation::Remainder,
            },
            lane_for_result(instruction, function)?,
            match semantics {
                KirArithmeticSemantics::Modular => KirCostSemantics::Modular,
                KirArithmeticSemantics::StrictFloat => KirCostSemantics::StrictFloat,
                KirArithmeticSemantics::Checked => return None,
            },
            KirAlignmentClass::NotApplicable,
        ),
        KirInstructionKind::Compare { .. } => (
            KirProfileOperation::Compare,
            KirLaneType::U32,
            KirCostSemantics::NotApplicable,
            KirAlignmentClass::NotApplicable,
        ),
        _ => return None,
    };
    operation_cost_key(
        profile,
        KirCostKey {
            operation,
            lane,
            lanes: 1,
            semantics,
            alignment,
        },
    )
}

fn operation_cost(
    profile: &crate::KirTargetProfile,
    operation: KirProfileOperation,
    lane: KirLaneType,
    semantics: KirCostSemantics,
) -> Option<u32> {
    operation_cost_key(
        profile,
        KirCostKey {
            operation,
            lane,
            lanes: 1,
            semantics,
            alignment: KirAlignmentClass::NotApplicable,
        },
    )
}

fn operation_cost_key(profile: &crate::KirTargetProfile, key: KirCostKey) -> Option<u32> {
    match profile.operation_availability(&key) {
        Some(KirOperationAvailability::Legal(cost)) if cost.legalization_parts == 1 => {
            Some(cost.cost)
        }
        Some(KirOperationAvailability::Unavailable)
            if key.operation == KirProfileOperation::Branch =>
        {
            Some(WASM_BRANCH_COST)
        }
        _ => None,
    }
}

fn lane_for_result(
    instruction: &KirInstruction,
    function: &crate::KirFunction,
) -> Option<KirLaneType> {
    let [result] = instruction.results.as_slice() else {
        return None;
    };
    let ty = value_type(function, result.value)?;
    lane_for_type(ty)
}

fn lane_for_type(ty: &MirType) -> Option<KirLaneType> {
    match ty {
        MirType::Primitive(MirPrimitiveTypeName::I32) => Some(KirLaneType::I32),
        MirType::Primitive(MirPrimitiveTypeName::U32) => Some(KirLaneType::U32),
        MirType::Primitive(MirPrimitiveTypeName::F64) => Some(KirLaneType::F64),
        _ => None,
    }
}

fn byte_width(ty: &MirType) -> Option<u16> {
    match ty {
        MirType::Primitive(MirPrimitiveTypeName::I32 | MirPrimitiveTypeName::U32) => Some(4),
        MirType::Primitive(MirPrimitiveTypeName::F64) => Some(8),
        _ => None,
    }
}

fn place_type(place: &KirPlace) -> Option<&MirType> {
    match place {
        KirPlace::SliceIndex { type_node, .. }
        | KirPlace::Index { type_node, .. }
        | KirPlace::Deref { type_node, .. }
        | KirPlace::Value { type_node, .. }
        | KirPlace::Field { type_node, .. } => Some(type_node),
    }
}

fn value_type(function: &crate::KirFunction, value: ValueId) -> Option<&MirType> {
    function
        .params
        .iter()
        .find_map(|parameter| (parameter.value == value).then_some(&parameter.type_node))
        .or_else(|| {
            function.blocks.iter().find_map(|block| {
                block.params.iter().find_map(|parameter| {
                    (parameter.value == value)
                        .then(|| parameter.type_node.as_scalar())
                        .flatten()
                })
            })
        })
        .or_else(|| {
            function
                .blocks
                .iter()
                .flat_map(|block| &block.instructions)
                .find_map(|instruction| {
                    instruction.results.iter().find_map(|result| {
                        (result.value == value)
                            .then(|| result.type_node.as_scalar())
                            .flatten()
                    })
                })
        })
}

fn constant_u32(function: &crate::KirFunction, value: ValueId) -> Option<u32> {
    function
        .blocks
        .iter()
        .flat_map(|block| &block.instructions)
        .find_map(|instruction| {
            let [result] = instruction.results.as_slice() else {
                return None;
            };
            if result.value != value {
                return None;
            }
            let KirInstructionKind::ConstInt { value } = &instruction.kind else {
                return None;
            };
            value.parse().ok()
        })
}

fn operand_right(kind: &KirInstructionKind) -> Option<ValueId> {
    match kind {
        KirInstructionKind::Binary { right, .. } => Some(*right),
        _ => None,
    }
}

fn type_is(ty: &crate::KirValueType, primitive: MirPrimitiveTypeName) -> bool {
    ty.as_scalar() == Some(&MirType::Primitive(primitive))
}

fn block(function: &crate::KirFunction, id: crate::BlockId) -> Option<&crate::KirBlock> {
    function.blocks.iter().find(|block| block.id == id)
}
