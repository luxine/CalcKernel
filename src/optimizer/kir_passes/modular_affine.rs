//! Shortens two fixed affine steps without changing any intermediate SSA value.

use std::collections::{BTreeMap, BTreeSet};

use crate::{
    BlockId, InstructionId, KirArithmeticSemantics, KirFunction, KirInstruction,
    KirInstructionKind, KirModule, KirOverflowMode, KirResult, KirTerminator, KirValueType,
    MirBinaryOp, MirPrimitiveTypeName, MirType, ValueId,
};

#[derive(Clone, Copy)]
struct Proposal {
    instruction: InstructionId,
    result: ValueId,
    root: ValueId,
    scale: u32,
    offset: u32,
    signed: bool,
}

type Definitions<'a> = BTreeMap<ValueId, (BlockId, &'a KirInstruction)>;
const MAX_COMPOSITIONS_PER_FUNCTION: usize = 64;

/// Restricts reassociation to the ring of 32-bit wrapping integers. Checked
/// arithmetic and strict floating point never enter this pass.
pub(crate) fn run_modular_affine_composition(
    module: &mut KirModule,
    protected: &BTreeSet<InstructionId>,
) -> bool {
    if module.config.overflow_mode != KirOverflowMode::Unchecked {
        return false;
    }
    let Some(mut next_value) =
        module
            .functions
            .iter()
            .flat_map(|function| {
                function.params.iter().map(|param| param.value).chain(
                    function.blocks.iter().flat_map(|block| {
                        block.params.iter().map(|param| param.value).chain(
                            block.instructions.iter().flat_map(|instruction| {
                                instruction.results.iter().map(|result| result.value)
                            }),
                        )
                    }),
                )
            })
            .map(ValueId::index)
            .max()
            .unwrap_or(0)
            .checked_add(1)
    else {
        return false;
    };
    let Some(mut next_instruction) = module
        .functions
        .iter()
        .flat_map(|function| &function.blocks)
        .flat_map(|block| &block.instructions)
        .map(|instruction| instruction.id.index())
        .max()
        .unwrap_or(0)
        .checked_add(1)
    else {
        return false;
    };
    let mut changed = false;
    for function in &mut module.functions {
        let proposals = discover(function, protected);
        if proposals.is_empty() {
            continue;
        }
        for block in &mut function.blocks {
            let mut instructions = Vec::with_capacity(block.instructions.len());
            for mut instruction in std::mem::take(&mut block.instructions) {
                if let Some(proposal) = proposals.get(&instruction.id)
                    && let (Some(value_end), Some(instruction_end)) =
                        (next_value.checked_add(3), next_instruction.checked_add(3))
                {
                    let ty = MirType::Primitive(if proposal.signed {
                        MirPrimitiveTypeName::I32
                    } else {
                        MirPrimitiveTypeName::U32
                    });
                    let scale = ValueId::from_index(next_value);
                    let offset = ValueId::from_index(next_value + 1);
                    let product = ValueId::from_index(next_value + 2);
                    let literal = |value: u32| {
                        if proposal.signed {
                            (value as i32).to_string()
                        } else {
                            value.to_string()
                        }
                    };
                    for (index, value, kind) in [
                        (
                            0,
                            scale,
                            KirInstructionKind::ConstInt {
                                value: literal(proposal.scale),
                            },
                        ),
                        (
                            1,
                            offset,
                            KirInstructionKind::ConstInt {
                                value: literal(proposal.offset),
                            },
                        ),
                        (
                            2,
                            product,
                            KirInstructionKind::Binary {
                                op: MirBinaryOp::Mul,
                                left: proposal.root,
                                right: scale,
                                semantics: KirArithmeticSemantics::Modular,
                            },
                        ),
                    ] {
                        instructions.push(KirInstruction {
                            id: InstructionId::from_index(next_instruction + index),
                            results: vec![KirResult {
                                value,
                                type_node: ty.clone().into(),
                            }],
                            kind,
                            memory: None,
                            effect: None,
                        });
                    }
                    // Existing definitions remain intact, even when their values
                    // are also used by a store, a branch or a second expression.
                    instruction.kind = KirInstructionKind::Binary {
                        op: MirBinaryOp::Add,
                        left: product,
                        right: offset,
                        semantics: KirArithmeticSemantics::Modular,
                    };
                    next_value = value_end;
                    next_instruction = instruction_end;
                    changed = true;
                }
                instructions.push(instruction);
            }
            block.instructions = instructions;
        }
    }
    changed
}

fn discover(
    function: &KirFunction,
    protected: &BTreeSet<InstructionId>,
) -> BTreeMap<InstructionId, Proposal> {
    let definitions = function
        .blocks
        .iter()
        .flat_map(|block| {
            block
                .instructions
                .iter()
                .filter(|instruction| instruction.results.len() == 1)
                .map(move |instruction| (instruction.results[0].value, (block.id, instruction)))
        })
        .collect::<Definitions<'_>>();
    let mut uses = BTreeMap::<ValueId, u8>::new();
    let mut count = |value| {
        let uses = uses.entry(value).or_default();
        *uses = uses.saturating_add(1);
    };
    for block in &function.blocks {
        for instruction in &block.instructions {
            for value in super::dce::instruction_uses(instruction) {
                count(value);
            }
        }
        match &block.terminator {
            KirTerminator::Return { value, .. } => {
                if let Some(value) = value {
                    count(*value);
                }
            }
            KirTerminator::Jump { edge } => edge.args.iter().copied().for_each(&mut count),
            KirTerminator::Branch {
                condition,
                then_edge,
                else_edge,
            } => {
                count(*condition);
                then_edge
                    .args
                    .iter()
                    .chain(&else_edge.args)
                    .copied()
                    .for_each(&mut count);
            }
        }
    }
    let mut proposals = BTreeMap::new();
    for block in &function.blocks {
        for instruction in &block.instructions {
            if proposals.len() == MAX_COMPOSITIONS_PER_FUNCTION {
                return proposals;
            }
            if protected.contains(&instruction.id) {
                continue;
            }
            let Some(result) = instruction.results.first() else {
                continue;
            };
            let signed = match result.type_node {
                KirValueType::Scalar(MirType::Primitive(MirPrimitiveTypeName::I32)) => true,
                KirValueType::Scalar(MirType::Primitive(MirPrimitiveTypeName::U32)) => false,
                _ => continue,
            };
            let step = |value, op| constant_rhs(&definitions, value, op, block.id, signed);
            let Some((outer_mul, d)) = step(result.value, MirBinaryOp::Add) else {
                continue;
            };
            // The new product replaces this multiplication only when DCE can
            // remove its old definition. Inner affine values may remain observable.
            if uses.get(&outer_mul) != Some(&1)
                || definitions
                    .get(&outer_mul)
                    .is_some_and(|(_, definition)| protected.contains(&definition.id))
            {
                continue;
            }
            let Some((inner_add, c)) = step(outer_mul, MirBinaryOp::Mul) else {
                continue;
            };
            let Some((inner_mul, b)) = step(inner_add, MirBinaryOp::Add) else {
                continue;
            };
            let Some((root, a)) = step(inner_mul, MirBinaryOp::Mul) else {
                continue;
            };
            let proposal = Proposal {
                instruction: instruction.id,
                result: result.value,
                root,
                scale: a.wrapping_mul(c),
                offset: b.wrapping_mul(c).wrapping_add(d),
                signed,
            };
            if check_proposal(function, &proposal) {
                proposals.insert(instruction.id, proposal);
            }
        }
    }
    proposals
}

fn constant_rhs(
    definitions: &Definitions<'_>,
    value: ValueId,
    expected: MirBinaryOp,
    block: BlockId,
    signed: bool,
) -> Option<(ValueId, u32)> {
    let (owner, instruction) = definitions.get(&value)?;
    if *owner != block || instruction.effect.is_some() || instruction.memory.is_some() {
        return None;
    }
    let KirInstructionKind::Binary {
        op,
        left,
        right,
        semantics,
    } = instruction.kind
    else {
        return None;
    };
    if op != expected || semantics != KirArithmeticSemantics::Modular {
        return None;
    }
    let (_, constant) = definitions.get(&right)?;
    let KirInstructionKind::ConstInt { value } = &constant.kind else {
        return None;
    };
    let integer = if signed {
        value.parse::<i32>().ok()? as u32
    } else {
        value.parse().ok()?
    };
    Some((left, integer))
}

/// Independently expand the original four operations as a polynomial over
/// Z/(2^32). This checker does not consume the discovery index or coefficients.
fn check_proposal(function: &KirFunction, proposal: &Proposal) -> bool {
    if proposal.root == proposal.result {
        return false;
    }
    let wanted_type = KirValueType::Scalar(MirType::Primitive(if proposal.signed {
        MirPrimitiveTypeName::I32
    } else {
        MirPrimitiveTypeName::U32
    }));
    let valid_root = function.params.iter().any(|param| {
        param.value == proposal.root && KirValueType::Scalar(param.type_node.clone()) == wanted_type
    }) || function.blocks.iter().any(|block| {
        block
            .params
            .iter()
            .any(|param| param.value == proposal.root && param.type_node == wanted_type)
            || block
                .instructions
                .iter()
                .flat_map(|instruction| &instruction.results)
                .any(|result| result.value == proposal.root && result.type_node == wanted_type)
    });
    if !valid_root
        || !function
            .blocks
            .iter()
            .flat_map(|block| &block.instructions)
            .any(|instruction| {
                instruction.id == proposal.instruction
                    && instruction.results.len() == 1
                    && instruction.results[0].value == proposal.result
            })
    {
        return false;
    }
    fn expand(
        function: &KirFunction,
        value: ValueId,
        root: ValueId,
        wanted_type: &KirValueType,
        budget: &mut u8,
    ) -> Option<(u32, u32)> {
        if *budget == 0 {
            return None;
        }
        *budget -= 1;
        if value == root {
            return Some((1, 0));
        }
        let instruction = function
            .blocks
            .iter()
            .flat_map(|block| &block.instructions)
            .find(|instruction| {
                instruction
                    .results
                    .iter()
                    .any(|result| result.value == value)
            })?;
        if instruction.results.len() != 1
            || instruction.results[0].type_node != *wanted_type
            || instruction.effect.is_some()
            || instruction.memory.is_some()
        {
            return None;
        }
        match &instruction.kind {
            KirInstructionKind::ConstInt { value } => {
                let bits = match wanted_type {
                    KirValueType::Scalar(MirType::Primitive(MirPrimitiveTypeName::I32)) => {
                        value.parse::<i32>().ok()? as u32
                    }
                    _ => value.parse::<u32>().ok()?,
                };
                Some((0, bits))
            }
            KirInstructionKind::Binary {
                op,
                left,
                right,
                semantics: KirArithmeticSemantics::Modular,
            } => {
                let (a, b) = expand(function, *left, root, wanted_type, budget)?;
                let (c, d) = expand(function, *right, root, wanted_type, budget)?;
                match op {
                    MirBinaryOp::Add => Some((a.wrapping_add(c), b.wrapping_add(d))),
                    MirBinaryOp::Mul if a == 0 || c == 0 => Some((
                        a.wrapping_mul(d).wrapping_add(b.wrapping_mul(c)),
                        b.wrapping_mul(d),
                    )),
                    _ => None,
                }
            }
            _ => None,
        }
    }
    expand(
        function,
        proposal.result,
        proposal.root,
        &wanted_type,
        &mut 9,
    ) == Some((proposal.scale, proposal.offset))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn module() -> KirModule {
        let checked = crate::check(&crate::SourceFile::new(
            "affine.ck",
            "export fn compose(x: u32) -> u32 { let y: u32 = x * 7 + 11; return y * 13 + 17; }",
        ));
        assert!(checked.diagnostics.is_empty());
        let mir = crate::lower_to_mir(&checked.checked_program).expect("MIR");
        crate::build_kir_module(
            &mir,
            crate::KirBuildConfig {
                consumer: crate::KirConsumer::Inspection,
                overflow_mode: KirOverflowMode::Unchecked,
                bounds_mode: crate::KirBoundsMode::Unchecked,
                sanitizer_mode: crate::KirSanitizerMode::Disabled,
            },
        )
        .expect("KIR")
    }

    #[test]
    fn checker_should_reject_tampered_coefficients_types_and_self_reference() {
        let module = module();
        let function = &module.functions[0];
        let proposal = *discover(function, &BTreeSet::new())
            .values()
            .next()
            .expect("proposal");
        assert!(check_proposal(function, &proposal));
        for tampered in [
            Proposal {
                scale: proposal.scale ^ 1,
                ..proposal
            },
            Proposal {
                offset: proposal.offset ^ 1,
                ..proposal
            },
            Proposal {
                signed: true,
                ..proposal
            },
            Proposal {
                root: proposal.result,
                scale: 1,
                offset: 0,
                ..proposal
            },
        ] {
            assert!(!check_proposal(function, &tampered));
        }
    }

    #[test]
    fn composition_should_preserve_proof_protected_instructions() {
        let mut module = module();
        let protected = discover(&module.functions[0], &BTreeSet::new())
            .keys()
            .copied()
            .collect();
        let before = module.clone();
        assert!(!run_modular_affine_composition(&mut module, &protected));
        assert_eq!(module, before);
    }

    #[test]
    fn checker_should_reject_effects_memory_and_checked_steps() {
        let original = module();
        let proposal = *discover(&original.functions[0], &BTreeSet::new())
            .values()
            .next()
            .expect("proposal");
        for mode in 0..3 {
            let mut function = original.functions[0].clone();
            let step = function
                .blocks
                .iter_mut()
                .flat_map(|block| &mut block.instructions)
                .find(|instruction| matches!(instruction.kind, KirInstructionKind::Binary { .. }))
                .expect("binary");
            match mode {
                0 => {
                    step.effect = Some(crate::KirOrderedEffect {
                        order: 0,
                        kind: crate::KirEffectKind::MayFail,
                    })
                }
                1 => {
                    step.memory = Some(crate::KirMemoryAccess {
                        region: crate::MemoryRegionId::from_index(0),
                        input: crate::MemoryVersionId::from_index(0),
                        output: None,
                    })
                }
                _ => {
                    if let KirInstructionKind::Binary { semantics, .. } = &mut step.kind {
                        *semantics = KirArithmeticSemantics::Checked;
                    }
                }
            }
            assert!(!check_proposal(&function, &proposal));
            assert!(discover(&function, &BTreeSet::new()).is_empty());
        }
    }
}
