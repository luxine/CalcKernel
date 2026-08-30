use std::collections::{BTreeMap, BTreeSet};

use crate::{
    BlockId, FunctionId, InstructionId, KirArithmeticSemantics, KirInstruction, KirInstructionKind,
    KirModule, KirPlace, KirTerminator, ValueId,
};

use super::super::NaturalLoopAnalysis;

#[derive(Debug, Default)]
pub(crate) struct LicmResult {
    pub hoisted: u32,
    pub exhausted_functions: Vec<FunctionId>,
}

/// Proposal-only queries, rebuilt before each loop so an earlier loop's
/// relocated/remapped instructions cannot leave stale Copy operands behind.
#[derive(Default)]
struct ForwardingIndex<'a> {
    parameters: BTreeMap<ValueId, (BlockId, usize)>,
    copies: BTreeMap<ValueId, ValueId>,
    incoming: BTreeMap<BlockId, Vec<&'a crate::KirEdge>>,
}

impl<'a> ForwardingIndex<'a> {
    fn new(function: &'a crate::KirFunction) -> Self {
        let mut queries = Self::default();
        for block in &function.blocks {
            for (index, param) in block.params.iter().enumerate() {
                queries.parameters.insert(param.value, (block.id, index));
            }
            for instruction in &block.instructions {
                if let KirInstructionKind::Copy { value } = instruction.kind
                    && let Some(result) = instruction.results.first()
                {
                    queries.copies.insert(result.value, value);
                }
            }
            for edge in edges(&block.terminator) {
                queries.incoming.entry(edge.target).or_default().push(edge);
            }
        }
        queries
    }
}

pub(crate) fn run_licm(
    module: &mut KirModule,
    protected: &BTreeSet<InstructionId>,
    analyses: &[NaturalLoopAnalysis],
) -> Result<LicmResult, String> {
    run_with_config(
        module,
        protected,
        analyses,
        super::super::ScalarAnalysisConfig::default(),
    )
}

fn run_with_config(
    module: &mut KirModule,
    protected: &BTreeSet<InstructionId>,
    analyses: &[NaturalLoopAnalysis],
    config: super::super::ScalarAnalysisConfig,
) -> Result<LicmResult, String> {
    let mut result = LicmResult::default();
    for (function, analysis) in module.functions.iter_mut().zip(analyses) {
        if analysis.loops.is_empty() {
            continue;
        }
        let original = function.clone();
        let mut remaining =
            super::super::ScalarAnalysisBudget::for_function(function, config).max_steps();
        let mut hoisted = 0_u32;
        let mut exhausted = false;
        let definitions = value_definitions(function);
        'loops: for loop_info in analysis.loops.iter().rev() {
            let loop_blocks = loop_info.blocks.iter().copied().collect::<BTreeSet<_>>();
            let preheaders = function
                .blocks
                .iter()
                .filter(|block| !loop_blocks.contains(&block.id))
                .filter(|block| {
                    edges(&block.terminator).any(|edge| edge.target == loop_info.header)
                })
                .map(|block| block.id)
                .collect::<Vec<_>>();
            let [preheader] = preheaders.as_slice() else {
                continue;
            };
            let queries = ForwardingIndex::new(function);
            let mut invariant_values = BTreeSet::new();
            let mut moved = Vec::<(BlockId, KirInstruction)>::new();
            let mut moved_ids = BTreeSet::new();
            let mut forwarding = BTreeMap::new();
            loop {
                let before = moved.len();
                for block in &function.blocks {
                    if !loop_blocks.contains(&block.id) || block.id == loop_info.header {
                        continue;
                    }
                    for instruction in &block.instructions {
                        let Some(next) = remaining.checked_sub(1) else {
                            exhausted = true;
                            break 'loops;
                        };
                        remaining = next;
                        if moved_ids.contains(&instruction.id)
                            || protected.contains(&instruction.id)
                            || !is_licm_pure(instruction)
                        {
                            continue;
                        }
                        let mut replacements = BTreeMap::new();
                        let mut can_move = true;
                        for value in instruction_uses(instruction) {
                            if invariant_values.contains(&value)
                                || defined_outside(&definitions, &loop_blocks, value)
                            {
                                continue;
                            }
                            let source = if let Some(source) = forwarding.get(&value) {
                                *source
                            } else {
                                let source = match forwarded_origin(
                                    &queries,
                                    value,
                                    &loop_blocks,
                                    &definitions,
                                    &mut remaining,
                                ) {
                                    Ok(source) => source,
                                    Err(()) => {
                                        exhausted = true;
                                        break 'loops;
                                    }
                                };
                                forwarding.insert(value, source);
                                source
                            };
                            let Some(source) = source else {
                                can_move = false;
                                break;
                            };
                            if !super::super::verify::verify_ssa_forwarding(function, value, source)
                            {
                                return Err("LICM forwarding claim does not match every SSA input"
                                    .to_string());
                            }
                            replacements.insert(value, source);
                        }
                        if can_move {
                            invariant_values
                                .extend(instruction.results.iter().map(|result| result.value));
                            let mut instruction = instruction.clone();
                            super::rewrite::remap_instruction_values(
                                &mut instruction,
                                &replacements,
                            );
                            moved_ids.insert(instruction.id);
                            moved.push((block.id, instruction));
                        }
                    }
                }
                if moved.len() == before {
                    break;
                }
            }
            if moved.is_empty() {
                continue;
            }
            for block in &mut function.blocks {
                block
                    .instructions
                    .retain(|instruction| !moved_ids.contains(&instruction.id));
            }
            if let Some(block) = function
                .blocks
                .iter_mut()
                .find(|block| block.id == *preheader)
            {
                block
                    .instructions
                    .extend(moved.into_iter().map(|(_, instruction)| instruction));
                hoisted =
                    hoisted.saturating_add(u32::try_from(moved_ids.len()).unwrap_or(u32::MAX));
            }
        }
        if exhausted {
            *function = original;
            result.exhausted_functions.push(function.id);
        } else {
            result.hoisted = result.hoisted.saturating_add(hoisted);
        }
    }
    Ok(result)
}

fn defined_outside(
    definitions: &BTreeMap<ValueId, Option<BlockId>>,
    loop_blocks: &BTreeSet<BlockId>,
    value: ValueId,
) -> bool {
    definitions
        .get(&value)
        .is_some_and(|block| block.is_none_or(|block| !loop_blocks.contains(&block)))
}

fn forwarded_origin(
    queries: &ForwardingIndex<'_>,
    value: ValueId,
    loop_blocks: &BTreeSet<BlockId>,
    definitions: &BTreeMap<ValueId, Option<BlockId>>,
    remaining: &mut u32,
) -> Result<Option<ValueId>, ()> {
    let mut pending = vec![value];
    let mut visited = BTreeSet::new();
    let mut origin = None;
    while let Some(value) = pending.pop() {
        if !visited.insert(value) {
            continue;
        }
        *remaining = remaining.checked_sub(1).ok_or(())?;
        if defined_outside(definitions, loop_blocks, value) {
            if origin.is_some_and(|origin| origin != value) {
                return Ok(None);
            }
            origin = Some(value);
        } else if let Some(&(block, index)) = queries.parameters.get(&value) {
            let Some(incoming) = queries.incoming.get(&block) else {
                return Ok(None);
            };
            for edge in incoming {
                let Some(value) = edge.args.get(index) else {
                    return Ok(None);
                };
                pending.push(*value);
            }
        } else if let Some(&operand) = queries.copies.get(&value) {
            pending.push(operand);
        } else {
            return Ok(None);
        }
    }
    Ok(origin)
}

#[cfg(test)]
fn linear_forwarded_origin(
    function: &crate::KirFunction,
    value: ValueId,
    loop_blocks: &BTreeSet<BlockId>,
    definitions: &BTreeMap<ValueId, Option<BlockId>>,
    remaining: &mut u32,
) -> Result<Option<ValueId>, ()> {
    let mut pending = vec![value];
    let mut visited = BTreeSet::new();
    let mut origin = None;
    while let Some(value) = pending.pop() {
        if !visited.insert(value) {
            continue;
        }
        *remaining = remaining.checked_sub(1).ok_or(())?;
        if defined_outside(definitions, loop_blocks, value) {
            if origin.is_some_and(|origin| origin != value) {
                return Ok(None);
            }
            origin = Some(value);
        } else if let Some((block, index)) = function.blocks.iter().find_map(|block| {
            block
                .params
                .iter()
                .position(|param| param.value == value)
                .map(|index| (block.id, index))
        }) {
            let mut incoming = false;
            for predecessor in &function.blocks {
                let edges = match &predecessor.terminator {
                    KirTerminator::Return { .. } => Vec::new(),
                    KirTerminator::Jump { edge } => vec![edge],
                    KirTerminator::Branch {
                        then_edge,
                        else_edge,
                        ..
                    } => vec![then_edge, else_edge],
                };
                for edge in edges.into_iter().filter(|edge| edge.target == block) {
                    let Some(value) = edge.args.get(index) else {
                        return Ok(None);
                    };
                    incoming = true;
                    pending.push(*value);
                }
            }
            if !incoming {
                return Ok(None);
            }
        } else if let Some(operand) = function
            .blocks
            .iter()
            .flat_map(|block| &block.instructions)
            .find_map(|instruction| {
                if instruction.results.first().map(|result| result.value) != Some(value) {
                    return None;
                }
                if let KirInstructionKind::Copy { value } = instruction.kind {
                    Some(value)
                } else {
                    None
                }
            })
        {
            pending.push(operand);
        } else {
            return Ok(None);
        }
    }
    Ok(origin)
}

fn is_licm_pure(instruction: &KirInstruction) -> bool {
    if instruction.effect.is_some()
        || instruction.memory.is_some()
        || instruction.results.is_empty()
    {
        return false;
    }
    match &instruction.kind {
        KirInstructionKind::ConstInt { .. }
        | KirInstructionKind::ConstBool { .. }
        | KirInstructionKind::Copy { .. }
        | KirInstructionKind::Compare { .. }
        | KirInstructionKind::Cast { .. } => true,
        KirInstructionKind::Binary { op, semantics, .. } => {
            *semantics == KirArithmeticSemantics::Modular
                && matches!(
                    op,
                    crate::MirBinaryOp::Add | crate::MirBinaryOp::Sub | crate::MirBinaryOp::Mul
                )
        }
        KirInstructionKind::Unary { semantics, .. } => {
            *semantics == KirArithmeticSemantics::Modular
        }
        _ => false,
    }
}

fn value_definitions(function: &crate::KirFunction) -> BTreeMap<ValueId, Option<BlockId>> {
    function
        .params
        .iter()
        .map(|param| (param.value, None))
        .chain(function.blocks.iter().flat_map(|block| {
            block
                .params
                .iter()
                .map(|param| (param.value, Some(block.id)))
                .chain(block.instructions.iter().flat_map(|instruction| {
                    instruction
                        .results
                        .iter()
                        .map(|result| (result.value, Some(block.id)))
                }))
        }))
        .collect()
}

fn edges(terminator: &KirTerminator) -> impl Iterator<Item = &crate::KirEdge> {
    let edges = match terminator {
        KirTerminator::Return { .. } => [None, None],
        KirTerminator::Jump { edge } => [Some(edge), None],
        KirTerminator::Branch {
            then_edge,
            else_edge,
            ..
        } => [Some(then_edge), Some(else_edge)],
    };
    edges.into_iter().flatten()
}

fn instruction_uses(instruction: &KirInstruction) -> Vec<ValueId> {
    match &instruction.kind {
        KirInstructionKind::Undef { .. }
        | KirInstructionKind::ConstInt { .. }
        | KirInstructionKind::ConstFloat { .. }
        | KirInstructionKind::ConstBool { .. } => Vec::new(),
        KirInstructionKind::Copy { value } | KirInstructionKind::Cast { value, .. } => vec![*value],
        KirInstructionKind::Binary { left, right, .. }
        | KirInstructionKind::Compare { left, right, .. } => vec![*left, *right],
        KirInstructionKind::Unary { operand, .. } => vec![*operand],
        KirInstructionKind::CheckCondition { args, .. }
        | KirInstructionKind::Call { args, .. }
        | KirInstructionKind::RuntimeCall { args, .. } => args.clone(),
        KirInstructionKind::Guard { condition, .. } => vec![*condition],
        KirInstructionKind::Address { place } | KirInstructionKind::Load { place } => {
            place_uses(place)
        }
        KirInstructionKind::Store { place, value } => {
            let mut values = place_uses(place);
            values.push(*value);
            values
        }
        KirInstructionKind::MakeSlice { data, len } => vec![*data, *len],
        KirInstructionKind::SliceData { slice } | KirInstructionKind::SliceLen { slice } => {
            vec![*slice]
        }
        KirInstructionKind::Subslice { slice, start, end } => vec![*slice, *start, *end],
    }
}

fn place_uses(place: &KirPlace) -> Vec<ValueId> {
    match place {
        KirPlace::Value { value, .. } => vec![*value],
        KirPlace::Deref { pointer, .. } => vec![*pointer],
        KirPlace::Index { base, index, .. } => {
            let mut values = place_uses(base);
            values.push(*index);
            values
        }
        KirPlace::SliceIndex { slice, index, .. } => vec![*slice, *index],
        KirPlace::Field { base, .. } => place_uses(base),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        KirBoundsMode, KirBuildConfig, KirConsumer, KirOverflowMode, KirSanitizerMode, SourceFile,
        build_kir_module, check, lower_to_mir,
    };

    fn fixture() -> KirModule {
        let checked = check(&SourceFile::new(
            "licm.ck",
            "export fn repeated(a: u32, b: u32, n: u32) -> u32 { let i: u32 = 0; let total: u32 = 0; while i < n { let scale: u32 = a * b; let shift: u32 = scale + a; total = total + shift; i = i + 1; } return total; }",
        ));
        assert!(checked.diagnostics.is_empty());
        build_kir_module(
            &lower_to_mir(&checked.checked_program).expect("MIR"),
            KirBuildConfig {
                consumer: KirConsumer::Inspection,
                overflow_mode: KirOverflowMode::Unchecked,
                bounds_mode: KirBoundsMode::Unchecked,
                sanitizer_mode: KirSanitizerMode::Disabled,
            },
        )
        .expect("KIR")
    }

    fn analyses(module: &KirModule) -> Vec<NaturalLoopAnalysis> {
        module
            .functions
            .iter()
            .map(super::super::super::analyze_natural_loops)
            .collect()
    }

    fn multiply(module: &KirModule) -> (BlockId, InstructionId) {
        module.functions[0]
            .blocks
            .iter()
            .find_map(|block| {
                block
                    .instructions
                    .iter()
                    .find(|instruction| {
                        matches!(
                            instruction.kind,
                            KirInstructionKind::Binary {
                                op: crate::MirBinaryOp::Mul,
                                ..
                            }
                        )
                    })
                    .map(|instruction| (block.id, instruction.id))
            })
            .expect("multiply")
    }

    #[test]
    fn licm_forwarding_should_consider_both_arms_to_the_same_header() {
        let mut module = fixture();
        let info = analyses(&module).remove(0);
        let function = &mut module.functions[0];
        let header = function
            .blocks
            .iter()
            .find(|block| block.id == info.loops[0].header)
            .expect("header");
        let position = header
            .params
            .iter()
            .position(|param| param.slot == "a")
            .expect("a");
        let value = header.params[position].value;
        let KirTerminator::Branch { condition, .. } = header.terminator else {
            panic!("condition")
        };
        let other = function.params[1].value;
        for block in &mut function.blocks {
            if info.loops[0].latches.contains(&block.id) {
                let KirTerminator::Jump { edge } = &block.terminator else {
                    panic!("latch")
                };
                let mut else_edge = edge.clone();
                else_edge.args[position] = other;
                block.terminator = KirTerminator::Branch {
                    condition,
                    then_edge: edge.clone(),
                    else_edge,
                };
            }
        }
        assert!(crate::validate_kir_module(&module).errors.is_empty());
        let function = &module.functions[0];
        let blocks = info.loops[0].blocks.iter().copied().collect();
        let definitions = value_definitions(function);
        let mut remaining = u32::MAX;
        assert_eq!(
            forwarded_origin(
                &ForwardingIndex::new(function),
                value,
                &blocks,
                &definitions,
                &mut remaining
            ),
            Ok(None)
        );
        assert_eq!(
            forwarded_origin(
                &ForwardingIndex::new(function),
                value,
                &blocks,
                &definitions,
                &mut 0
            ),
            Err(())
        );
    }

    #[test]
    fn licm_forwarding_index_should_preserve_linear_results_and_budget() {
        let module = fixture();
        let function = &module.functions[0];
        let info = analyses(&module).remove(0);
        let definitions = value_definitions(function);
        let queries = ForwardingIndex::new(function);
        for loop_info in &info.loops {
            let blocks = loop_info.blocks.iter().copied().collect();
            for value in definitions
                .keys()
                .copied()
                .chain(std::iter::once(ValueId::from_index(u32::MAX)))
            {
                for limit in 0..100 {
                    let mut indexed_budget = limit;
                    let mut linear_budget = limit;
                    assert_eq!(
                        forwarded_origin(
                            &queries,
                            value,
                            &blocks,
                            &definitions,
                            &mut indexed_budget
                        ),
                        linear_forwarded_origin(
                            function,
                            value,
                            &blocks,
                            &definitions,
                            &mut linear_budget
                        )
                    );
                    assert_eq!(indexed_budget, linear_budget);
                }
            }
        }
    }

    #[test]
    fn licm_forwarding_checker_should_reject_a_changed_backedge_and_wrong_source() {
        let mut module = fixture();
        let info = analyses(&module).remove(0);
        let function = &mut module.functions[0];
        let header = function
            .blocks
            .iter()
            .find(|block| block.id == info.loops[0].header)
            .expect("header");
        let index = header
            .params
            .iter()
            .position(|param| param.slot == "a")
            .expect("a phi");
        let value = header.params[index].value;
        let source = function.params[0].value;
        let other = function.params[1].value;
        assert!(super::super::super::verify::verify_ssa_forwarding(
            function, value, source
        ));
        assert!(!super::super::super::verify::verify_ssa_forwarding(
            function, value, other
        ));
        for block in &mut function.blocks {
            if info.loops[0].latches.contains(&block.id) {
                let KirTerminator::Jump { edge } = &mut block.terminator else {
                    panic!("latch");
                };
                edge.args[index] = other;
            }
        }
        assert!(crate::validate_kir_module(&module).errors.is_empty());
        assert!(!super::super::super::verify::verify_ssa_forwarding(
            &module.functions[0],
            value,
            source
        ));
    }

    #[test]
    fn licm_should_preserve_protected_producer_and_dependency_order() {
        let original = fixture();
        let protected = multiply(&original);
        let mut module = original.clone();
        let info = analyses(&module);
        run_licm(&mut module, &BTreeSet::from([protected.1]), &info).expect("protected pass");
        assert_eq!(multiply(&module), protected);
        assert!(crate::validate_kir_module(&module).errors.is_empty());

        let mut module = original;
        for instruction in module.functions[0]
            .blocks
            .iter_mut()
            .flat_map(|block| &mut block.instructions)
        {
            instruction.id = InstructionId::from_index(1000 - instruction.id.index());
        }
        module.functions[0].blocks[1..].reverse();
        assert!(crate::validate_kir_module(&module).errors.is_empty());
        let info = analyses(&module);
        let result = run_licm(&mut module, &BTreeSet::new(), &info).expect("pass");
        assert!(result.hoisted >= 2);
        assert_eq!(multiply(&module).0, module.functions[0].blocks[0].id);
        let validation = crate::validate_kir_module(&module);
        assert!(validation.errors.is_empty(), "{:?}", validation.errors);
    }

    #[test]
    fn licm_budget_should_discard_the_whole_function_transaction() {
        let original = fixture();
        let info = analyses(&original);
        let maximum = super::super::super::ScalarAnalysisBudget::for_function(
            &original.functions[0],
            super::super::super::ScalarAnalysisConfig::default(),
        )
        .max_steps();
        let mut exhausted = false;
        let mut successful = false;
        for limit in (0..=maximum).step_by(3).chain(std::iter::once(maximum)) {
            let mut module = original.clone();
            let result = run_with_config(
                &mut module,
                &BTreeSet::new(),
                &info,
                super::super::super::ScalarAnalysisConfig::with_max_steps(limit),
            )
            .expect("budget fallback");
            if result.exhausted_functions.is_empty() {
                successful = true;
                assert_eq!(multiply(&module).0, module.functions[0].blocks[0].id);
            } else {
                exhausted = true;
                assert_eq!(result.hoisted, 0);
                assert_eq!(module, original);
            }
        }
        assert!(successful && exhausted);
    }
}
