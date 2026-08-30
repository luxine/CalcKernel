use std::collections::{BTreeMap, BTreeSet};

use num_bigint::BigInt;

use crate::{
    BlockId, FunctionId, InstructionId, KirArithmeticSemantics, KirFunction, KirInstruction,
    KirInstructionKind, KirModule, KirResult, KirTerminator, MirBinaryOp, ValueId,
};

use super::super::{
    FactArena, FactUseSite, IntegerType, NaturalLoopAnalysis, ProofArena, ProofStep, ProofStepId,
    ScalarAnalysisBudget, ScalarAnalysisConfig, ScalarValue, verify_proof_arena,
};

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct InductionSimplification {
    pub simplified: u32,
    pub exhausted_functions: Vec<FunctionId>,
}

#[derive(Debug, Clone)]
struct Replacement {
    function: FunctionId,
    block: BlockId,
    value: ValueId,
    source: ValueId,
    proof: crate::ProofId,
}

/// Read-only proposal queries for one immutable function pre-state. The proof
/// checker deliberately does not consume this optimization-side index.
#[derive(Default)]
struct InductionIndex<'a> {
    blocks: BTreeMap<BlockId, &'a crate::KirBlock>,
    parameters: BTreeMap<ValueId, (BlockId, usize)>,
    instructions: BTreeMap<ValueId, &'a KirInstruction>,
    integer_types: BTreeMap<ValueId, IntegerType>,
    incoming: BTreeMap<BlockId, Vec<&'a crate::KirEdge>>,
}

impl<'a> InductionIndex<'a> {
    fn new(function: &'a KirFunction) -> Self {
        let mut index = Self::default();
        for param in &function.params {
            if let Some(ty) = IntegerType::from_mir(&param.type_node) {
                index.integer_types.insert(param.value, ty);
            }
        }
        for block in &function.blocks {
            index.blocks.insert(block.id, block);
            for (position, param) in block.params.iter().enumerate() {
                index.parameters.insert(param.value, (block.id, position));
                if let Some(ty) = IntegerType::from_mir(&param.type_node) {
                    index.integer_types.insert(param.value, ty);
                }
            }
            for instruction in &block.instructions {
                if let Some(result) = instruction.results.first() {
                    index.instructions.insert(result.value, instruction);
                    if let Some(ty) = IntegerType::from_mir(&result.type_node) {
                        index.integer_types.insert(result.value, ty);
                    }
                }
            }
            for edge in edges(&block.terminator) {
                index.incoming.entry(edge.target).or_default().push(edge);
            }
        }
        index
    }
}

pub(crate) fn run_induction_simplification(
    module: &mut KirModule,
    live_proofs: &ProofArena,
    analyses: &[NaturalLoopAnalysis],
) -> Result<InductionSimplification, String> {
    run_with_config(
        module,
        live_proofs,
        analyses,
        ScalarAnalysisConfig::default(),
    )
}

fn run_with_config(
    module: &mut KirModule,
    live_proofs: &ProofArena,
    analyses: &[NaturalLoopAnalysis],
    config: ScalarAnalysisConfig,
) -> Result<InductionSimplification, String> {
    let protected = live_proofs.block_parameter_dependencies();
    let mut proofs = ProofArena::new(live_proofs.generation());
    let mut replacements = Vec::new();
    let mut result = InductionSimplification::default();
    for (function, analysis) in module.functions.iter().zip(analyses) {
        if analysis.loops.is_empty() {
            continue;
        }
        let queries = InductionIndex::new(function);
        let mut remaining = ScalarAnalysisBudget::for_function(function, config).max_steps();
        let mut pending = BTreeMap::new();
        let mut certificates = Vec::new();
        let mut exhausted = false;
        'loops: for loop_info in &analysis.loops {
            let Some(header) = queries.blocks.get(&loop_info.header) else {
                return Err("induction analysis names a missing header".to_string());
            };
            for (index, left) in header.params.iter().enumerate() {
                for right in header.params.iter().skip(index + 1) {
                    let (left, right) = (left.value.min(right.value), left.value.max(right.value));
                    if protected.contains(&right) || pending.contains_key(&right) {
                        continue;
                    }
                    let proposal =
                        match propose_equality(&queries, header.id, left, right, &mut remaining) {
                            Ok(Some(proposal)) => proposal,
                            Ok(None) => continue,
                            Err(()) => {
                                exhausted = true;
                                break 'loops;
                            }
                        };
                    let ProofStep::InductionEquality { pairs, .. } = &proposal else {
                        return Err(
                            "induction proposal has an unexpected certificate kind".to_string()
                        );
                    };
                    let certificate_index = certificates.len();
                    for &(source, value) in pairs {
                        if protected.contains(&value) || pending.contains_key(&value) {
                            continue;
                        }
                        if let (Some((source_block, _)), Some((block, _))) = (
                            queries.parameters.get(&source).copied(),
                            queries.parameters.get(&value).copied(),
                        ) && source_block == block
                        {
                            pending.insert(value, (block, source, certificate_index));
                        }
                    }
                    certificates.push((header.id, proposal));
                }
            }
        }
        if exhausted {
            result.exhausted_functions.push(function.id);
            continue;
        }
        let mut ids = Vec::new();
        for (header, step) in certificates {
            ids.push(
                proofs
                    .try_insert(
                        FactUseSite {
                            function: function.id,
                            block: header,
                            instruction: None,
                            contract_instance: None,
                        },
                        vec![step],
                        ProofStepId::from_index(0),
                    )
                    .map_err(|error| error.to_string())?,
            );
        }
        replacements.extend(
            pending
                .into_iter()
                .map(|(value, (block, source, certificate))| Replacement {
                    function: function.id,
                    block,
                    value,
                    source,
                    proof: ids[certificate],
                }),
        );
    }
    result.simplified = apply_replacements(module, &proofs, &replacements)?;
    Ok(result)
}

fn propose_equality(
    queries: &InductionIndex<'_>,
    header: BlockId,
    left: ValueId,
    right: ValueId,
    remaining: &mut u32,
) -> Result<Option<ProofStep>, ()> {
    let mut pending = vec![(left, right)];
    let mut pairs = BTreeSet::new();
    let mut definitions = BTreeSet::new();
    while let Some((a, b)) = pending.pop() {
        if a == b {
            continue;
        }
        let (a, b) = (a.min(b), a.max(b));
        if !pairs.insert((a, b)) {
            continue;
        }
        *remaining = remaining.checked_sub(1).ok_or(())?;
        let Some(ty) = queries.integer_types.get(&a).copied() else {
            return Ok(None);
        };
        if queries.integer_types.get(&b).copied() != Some(ty) {
            return Ok(None);
        }
        let a_instruction = queries.instructions.get(&a).copied();
        let b_instruction = queries.instructions.get(&b).copied();
        definitions.extend(
            a_instruction
                .iter()
                .chain(b_instruction.iter())
                .map(|instruction| instruction.id),
        );
        if let Some(KirInstruction {
            kind: KirInstructionKind::Copy { value },
            ..
        }) = a_instruction
        {
            pending.push((*value, b));
            continue;
        }
        if let Some(KirInstruction {
            kind: KirInstructionKind::Copy { value },
            ..
        }) = b_instruction
        {
            pending.push((a, *value));
            continue;
        }
        if let (Some((a_block, a_index)), Some((b_block, b_index))) = (
            queries.parameters.get(&a).copied(),
            queries.parameters.get(&b).copied(),
        ) {
            if a_block != b_block {
                return Ok(None);
            }
            let Some(incoming) = queries.incoming.get(&a_block) else {
                return Ok(None);
            };
            for edge in incoming {
                let (Some(a), Some(b)) = (edge.args.get(a_index), edge.args.get(b_index)) else {
                    return Ok(None);
                };
                pending.push((*a, *b));
            }
            continue;
        }
        let (Some(a), Some(b)) = (a_instruction, b_instruction) else {
            return Ok(None);
        };
        if a.memory.is_some() || b.memory.is_some() || a.effect.is_some() || b.effect.is_some() {
            return Ok(None);
        }
        match (&a.kind, &b.kind) {
            (
                KirInstructionKind::ConstInt { value: a },
                KirInstructionKind::ConstInt { value: b },
            ) => {
                let (Ok(a), Ok(b)) = (a.parse::<BigInt>(), b.parse::<BigInt>()) else {
                    return Ok(None);
                };
                if a != b || ScalarValue::constant(ty, a).is_err() {
                    return Ok(None);
                }
            }
            (
                KirInstructionKind::Binary {
                    op: a_op,
                    left: a_left,
                    right: a_right,
                    semantics: a_semantics,
                },
                KirInstructionKind::Binary {
                    op: b_op,
                    left: b_left,
                    right: b_right,
                    semantics: b_semantics,
                },
            ) => {
                if !matches!(a_op, MirBinaryOp::Add | MirBinaryOp::Sub)
                    || a_op != b_op
                    || a_semantics != b_semantics
                    || *a_semantics == KirArithmeticSemantics::StrictFloat
                {
                    return Ok(None);
                }
                pending.extend([(*a_left, *b_left), (*a_right, *b_right)]);
            }
            _ => return Ok(None),
        }
    }
    Ok(Some(ProofStep::InductionEquality {
        header,
        left,
        right,
        pairs: pairs.into_iter().collect(),
        definitions: definitions.into_iter().collect(),
    }))
}

fn apply_replacements(
    module: &mut KirModule,
    proofs: &ProofArena,
    replacements: &[Replacement],
) -> Result<u32, String> {
    let validation = verify_proof_arena(
        module,
        &FactArena::new(proofs.generation()),
        None,
        proofs,
        proofs.generation(),
    );
    if let Some(error) = validation.errors.first() {
        return Err(format!("invalid induction certificate: {}", error.message));
    }
    if replacements.is_empty() {
        return Ok(0);
    }
    let count = u32::try_from(replacements.len()).map_err(|_| "too many induction replacements")?;
    let first = module
        .functions
        .iter()
        .flat_map(|function| &function.blocks)
        .flat_map(|block| &block.instructions)
        .map(|instruction| instruction.id.index())
        .max()
        .map_or(Some(0), |id| id.checked_add(1))
        .ok_or("induction instruction identity space exhausted")?;
    first
        .checked_add(count - 1)
        .ok_or("induction instruction identity space exhausted")?;
    let mut prepared = BTreeMap::<(FunctionId, BlockId), BTreeMap<usize, KirInstruction>>::new();
    for (offset, replacement) in replacements.iter().enumerate() {
        let proof = proofs
            .get(replacement.proof)
            .ok_or("missing induction proof")?;
        let Some(ProofStep::InductionEquality { pairs, .. }) =
            proof.steps.get(proof.root.index() as usize)
        else {
            return Err("induction replacement has wrong proof kind".to_string());
        };
        if proof.use_site.function != replacement.function
            || pairs
                .binary_search(&(replacement.source, replacement.value))
                .is_err()
        {
            return Err("induction replacement does not match its certificate".to_string());
        }
        let function = module
            .functions
            .iter()
            .find(|function| function.id == replacement.function)
            .ok_or("missing induction function")?;
        let block = function
            .blocks
            .iter()
            .find(|block| block.id == replacement.block)
            .ok_or("missing induction block")?;
        let (index, param) = block
            .params
            .iter()
            .enumerate()
            .find(|(_, param)| param.value == replacement.value)
            .ok_or("missing induction parameter")?;
        if !block
            .params
            .iter()
            .any(|param| param.value == replacement.source)
        {
            return Err("induction source is not a same-block parameter".to_string());
        }
        for predecessor in &function.blocks {
            if edges(&predecessor.terminator)
                .any(|edge| edge.target == block.id && edge.args.len() != block.params.len())
            {
                return Err("induction replacement has incomplete incoming arguments".to_string());
            }
        }
        let instruction = KirInstruction {
            id: InstructionId::from_index(first + offset as u32),
            results: vec![KirResult {
                value: param.value,
                type_node: param.type_node.clone(),
            }],
            kind: KirInstructionKind::Copy {
                value: replacement.source,
            },
            memory: None,
            effect: None,
        };
        if prepared
            .entry((function.id, block.id))
            .or_default()
            .insert(index, instruction)
            .is_some()
        {
            return Err("duplicate induction replacement".to_string());
        }
    }
    // All certificates, replacement identities, edges and fresh IDs have been
    // checked against the immutable pre-state. Keep ValueIds for live evidence.
    for function in &mut module.functions {
        for block in &mut function.blocks {
            match &mut block.terminator {
                KirTerminator::Return { .. } => {}
                KirTerminator::Jump { edge } => repair_edge(function.id, edge, &prepared),
                KirTerminator::Branch {
                    then_edge,
                    else_edge,
                    ..
                } => {
                    repair_edge(function.id, then_edge, &prepared);
                    repair_edge(function.id, else_edge, &prepared);
                }
            }
            if let Some(replacements) = prepared.get(&(function.id, block.id)) {
                let mut index = 0;
                block.params.retain(|_| {
                    let keep = !replacements.contains_key(&index);
                    index += 1;
                    keep
                });
                let mut copies = replacements.values().cloned().collect::<Vec<_>>();
                copies.sort_by_key(|instruction| instruction.results[0].value);
                block.instructions.splice(0..0, copies);
            }
        }
    }
    Ok(count)
}

fn repair_edge(
    function: FunctionId,
    edge: &mut crate::KirEdge,
    prepared: &BTreeMap<(FunctionId, BlockId), BTreeMap<usize, KirInstruction>>,
) {
    if let Some(replacements) = prepared.get(&(function, edge.target)) {
        let mut index = 0;
        edge.args.retain(|_| {
            let keep = !replacements.contains_key(&index);
            index += 1;
            keep
        });
    }
}

#[cfg(test)]
fn parameter(function: &KirFunction, value: ValueId) -> Option<(BlockId, usize)> {
    function.blocks.iter().find_map(|block| {
        block
            .params
            .iter()
            .position(|param| param.value == value)
            .map(|index| (block.id, index))
    })
}

#[cfg(test)]
fn defining_instruction(function: &KirFunction, value: ValueId) -> Option<&KirInstruction> {
    function
        .blocks
        .iter()
        .flat_map(|block| &block.instructions)
        .find(|instruction| {
            instruction
                .results
                .first()
                .is_some_and(|result| result.value == value)
        })
}

#[cfg(test)]
fn integer_type(function: &KirFunction, value: ValueId) -> Option<IntegerType> {
    function
        .params
        .iter()
        .find(|param| param.value == value)
        .and_then(|param| IntegerType::from_mir(&param.type_node))
        .or_else(|| {
            function
                .blocks
                .iter()
                .flat_map(|block| &block.params)
                .find(|param| param.value == value)
                .and_then(|param| IntegerType::from_mir(&param.type_node))
        })
        .or_else(|| {
            defining_instruction(function, value)
                .and_then(|instruction| IntegerType::from_mir(&instruction.results[0].type_node))
        })
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        KirBoundsMode, KirBuildConfig, KirConsumer, KirOverflowMode, KirSanitizerMode, SourceFile,
        build_kir_module, check, lower_to_mir,
    };

    fn module() -> KirModule {
        let checked = check(&SourceFile::new(
            "induction.ck",
            "export fn count(n: u32) -> u32 { let i: u32 = 0; let j: u32 = 0; let k: u32 = 0; while i < n { i = i + 2; j = j + 2; k = k + 2; } return j + k; }",
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

    fn proof(module: &KirModule) -> (ProofArena, Replacement) {
        let function = &module.functions[0];
        let header = super::super::super::analyze_natural_loops(function).loops[0].header;
        let block = function
            .blocks
            .iter()
            .find(|block| block.id == header)
            .expect("header");
        let left = block
            .params
            .iter()
            .find(|param| param.slot == "i")
            .expect("i")
            .value;
        let right = block
            .params
            .iter()
            .find(|param| param.slot == "j")
            .expect("j")
            .value;
        let mut remaining = u32::MAX;
        let step = propose_equality(
            &InductionIndex::new(function),
            header,
            left,
            right,
            &mut remaining,
        )
        .expect("budget")
        .expect("equal induction");
        let mut proofs = ProofArena::new(0);
        let proof = proofs
            .try_insert(
                FactUseSite {
                    function: function.id,
                    block: header,
                    instruction: None,
                    contract_instance: None,
                },
                vec![step],
                ProofStepId::from_index(0),
            )
            .expect("certificate");
        (
            proofs,
            Replacement {
                function: function.id,
                block: header,
                value: right,
                source: left,
                proof,
            },
        )
    }

    #[test]
    fn induction_index_should_match_linear_queries_and_keep_both_branch_arms() {
        let mut module = module();
        for block in &mut module.functions[0].blocks {
            if let KirTerminator::Branch {
                then_edge,
                else_edge,
                ..
            } = &mut block.terminator
            {
                *else_edge = then_edge.clone();
            }
        }
        let function = &module.functions[0];
        let queries = InductionIndex::new(function);
        let values = function
            .params
            .iter()
            .map(|param| param.value)
            .chain(
                function
                    .blocks
                    .iter()
                    .flat_map(|block| block.params.iter().map(|param| param.value)),
            )
            .chain(function.blocks.iter().flat_map(|block| {
                block
                    .instructions
                    .iter()
                    .flat_map(|instruction| instruction.results.iter().map(|result| result.value))
            }))
            .chain(std::iter::once(ValueId::from_index(u32::MAX)));
        for value in values {
            assert_eq!(
                queries.parameters.get(&value).copied(),
                parameter(function, value)
            );
            assert_eq!(
                queries.instructions.get(&value).copied(),
                defining_instruction(function, value)
            );
            assert_eq!(
                queries.integer_types.get(&value).copied(),
                integer_type(function, value)
            );
        }
        for block in &function.blocks {
            let incoming = function
                .blocks
                .iter()
                .flat_map(|predecessor| edges(&predecessor.terminator))
                .filter(|edge| edge.target == block.id)
                .collect::<Vec<_>>();
            assert_eq!(
                queries
                    .incoming
                    .get(&block.id)
                    .map(Vec::as_slice)
                    .unwrap_or_default(),
                incoming
            );
            if let KirTerminator::Branch {
                then_edge,
                else_edge,
                ..
            } = &block.terminator
            {
                assert_eq!(then_edge.target, else_edge.target);
                let actual = &queries.incoming[&then_edge.target];
                assert!(actual.iter().any(|edge| std::ptr::eq(*edge, then_edge)));
                assert!(actual.iter().any(|edge| std::ptr::eq(*edge, else_edge)));
            }
        }
    }

    #[test]
    fn induction_certificate_should_reject_missing_transfer_pair_and_definition() {
        let original = module();
        let (proofs, replacement) = proof(&original);
        assert!(
            verify_proof_arena(&original, &FactArena::new(0), None, &proofs, 0)
                .errors
                .is_empty()
        );
        for missing_definition in [false, true] {
            let mut module = original.clone();
            let mut proofs = proofs.clone();
            let ProofStep::InductionEquality {
                pairs, definitions, ..
            } = &mut proofs.get_mut(replacement.proof).expect("proof").steps[0]
            else {
                panic!("equality");
            };
            if missing_definition {
                definitions.pop();
            } else {
                pairs.pop();
            }
            assert!(
                apply_replacements(&mut module, &proofs, std::slice::from_ref(&replacement))
                    .is_err()
            );
            assert_eq!(module, original);
        }
    }

    #[test]
    fn induction_certificate_should_reject_changed_initial_value_and_step() {
        for initial in [true, false] {
            let mut module = module();
            let (proofs, replacement) = proof(&module);
            let function = &mut module.functions[0];
            let index = function
                .blocks
                .iter()
                .find(|block| block.id == replacement.block)
                .expect("header")
                .params
                .iter()
                .position(|param| param.value == replacement.value)
                .expect("j");
            let value = function
                .blocks
                .iter()
                .find_map(|block| match &block.terminator {
                    KirTerminator::Jump { edge }
                        if edge.target == replacement.block
                            && (block.id == function.blocks[0].id) == initial =>
                    {
                        Some(edge.args[index])
                    }
                    _ => None,
                })
                .expect("incoming value");
            let instruction = function
                .blocks
                .iter_mut()
                .flat_map(|block| &mut block.instructions)
                .find(|instruction| instruction.results[0].value == value)
                .expect("definition");
            if initial {
                instruction.kind = KirInstructionKind::ConstInt {
                    value: "1".to_string(),
                };
            } else {
                let KirInstructionKind::Binary { op, .. } = &mut instruction.kind else {
                    panic!("transfer");
                };
                *op = MirBinaryOp::Sub;
            }
            assert!(crate::validate_kir_module(&module).errors.is_empty());
            let before = module.clone();
            assert!(apply_replacements(&mut module, &proofs, &[replacement]).is_err());
            assert_eq!(module, before);
        }
    }

    #[test]
    fn induction_replacement_should_reject_a_wrong_target_atomically() {
        let mut module = module();
        let original = module.clone();
        let (proofs, replacement) = proof(&module);
        let mut wrong = replacement.clone();
        wrong.value = module.functions[0].params[0].value;
        assert!(apply_replacements(&mut module, &proofs, &[replacement, wrong]).is_err());
        assert_eq!(module, original);
    }

    #[test]
    fn induction_replacement_should_reject_exhausted_instruction_ids_atomically() {
        let mut module = module();
        module.functions[0]
            .blocks
            .last_mut()
            .expect("last block")
            .instructions
            .last_mut()
            .expect("return addition")
            .id = InstructionId::from_index(u32::MAX);
        let original = module.clone();
        let (proofs, replacement) = proof(&module);
        assert!(
            apply_replacements(&mut module, &proofs, &[replacement])
                .expect_err("no IDs")
                .contains("identity space exhausted")
        );
        assert_eq!(module, original);
    }

    #[test]
    fn induction_budget_should_discard_all_pending_function_rewrites() {
        let original = module();
        let analyses = original
            .functions
            .iter()
            .map(super::super::super::analyze_natural_loops)
            .collect::<Vec<_>>();
        let mut saw_exhaustion = false;
        let mut saw_success = false;
        for budget in 0..100 {
            let mut module = original.clone();
            let result = run_with_config(
                &mut module,
                &ProofArena::new(0),
                &analyses,
                ScalarAnalysisConfig::with_max_steps(budget),
            )
            .expect("budget fallback");
            if result.exhausted_functions.is_empty() {
                assert!(result.simplified > 0);
                saw_success = true;
            } else {
                assert_eq!(module, original, "partial mutation at budget {budget}");
                assert_eq!(result.simplified, 0);
                saw_exhaustion = true;
            }
        }
        assert!(saw_exhaustion && saw_success);
    }

    #[test]
    fn induction_replacement_should_preserve_live_phi_certificate_dependencies() {
        let mut module = module();
        let (proofs, _) = proof(&module);
        let analyses = module
            .functions
            .iter()
            .map(super::super::super::analyze_natural_loops)
            .collect::<Vec<_>>();
        run_induction_simplification(&mut module, &proofs, &analyses).expect("protected induction");
        assert!(crate::validate_kir_module(&module).errors.is_empty());
        assert!(
            verify_proof_arena(&module, &FactArena::new(0), None, &proofs, 0)
                .errors
                .is_empty()
        );
    }
}
