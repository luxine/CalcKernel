//! Reconstructs the address identity from source KIR, without calling discovery.
use super::factored::FactoredAddressPlan;
use crate::{
    BlockId, KirArithmeticSemantics, KirFunction, KirInstruction, KirInstructionKind, KirPlace,
    KirTerminator, MirBinaryOp, MirPrimitiveTypeName, MirType, ValueId,
};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) fn validate(function: &KirFunction, plan: &FactoredAddressPlan) -> bool {
    if !super::factored_bundle_check::validate(function, &plan.bundles) {
        return false;
    }
    if plan.bases.is_empty() && plan.accesses.is_empty() {
        return true;
    }
    if function.blocks.len() > 256 || plan.bases.len() > 16 || plan.accesses.len() > 64 {
        return false;
    }
    let loops = crate::analyze_canonical_loops(function);
    let dominators = crate::compute_kir_dominators(function);
    let mut bases = BTreeMap::new();
    for base in &plan.bases {
        if base.id >= 16
            || bases.insert(base.id, base).is_some()
            || !matches!(base.element_bytes, 4 | 8)
        {
            return false;
        }
        let Some(descriptor) = loops.loops.iter().find(|loop_| loop_.header == base.header) else {
            return false;
        };
        if !descriptor.innermost
            || !descriptor.lcssa
            || !descriptor.dedicated_exits
            || !(3..=9).contains(&descriptor.blocks.len())
            || descriptor.preheader != Some(base.preheader)
            || descriptor.exits.len() != 1
            || descriptor.latch.is_none()
        {
            return false;
        }
        let Some(induction) = &descriptor.induction else {
            return false;
        };
        if induction.start != 0.into()
            || induction.step != 1.into()
            || induction.comparison != crate::MirCompareOp::Lt
            || type_of(function, induction.value)
                != Some(&MirType::Primitive(MirPrimitiveTypeName::U32))
            || type_of(function, base.offset)
                != Some(&MirType::Primitive(MirPrimitiveTypeName::U32))
        {
            return false;
        }
        let width = match type_of(function, base.slice) {
            Some(MirType::Slice(element)) => element_width(element),
            _ => None,
        };
        if width != Some(base.element_bytes) {
            return false;
        }
        let members = descriptor.blocks.iter().copied().collect::<BTreeSet<_>>();
        if !available_at_entry(function, &dominators, &members, base.preheader, base.offset)
            || !available_at_entry(function, &dominators, &members, base.preheader, base.slice)
        {
            return false;
        }
        let Some(preheader) = function
            .blocks
            .iter()
            .find(|block| block.id == base.preheader)
        else {
            return false;
        };
        if !matches!(&preheader.terminator, KirTerminator::Jump { edge } if edge.target == base.header)
        {
            return false;
        }
        let mut entries = 0;
        let mut latches = 0;
        let mut memory_defs = BTreeSet::new();
        for block in function
            .blocks
            .iter()
            .filter(|block| members.contains(&block.id))
        {
            if !dominators.dominates(base.header, block.id) {
                return false;
            }
            memory_defs.extend(block.memory_params.iter().map(|p| p.version));
            memory_defs.extend(
                block
                    .instructions
                    .iter()
                    .filter_map(|i| i.memory.as_ref().and_then(|m| m.output)),
            );
            let edges = successors(&block.terminator);
            if edges.is_empty() {
                return false;
            }
            for edge in edges {
                if edge.target == base.header {
                    if Some(block.id) != descriptor.latch
                        || !matches!(block.terminator, KirTerminator::Jump { .. })
                    {
                        return false;
                    }
                    latches += 1;
                } else if !members.contains(&edge.target)
                    && (block.id != base.header || edge.target != descriptor.exits[0])
                {
                    return false;
                }
            }
        }
        for block in function
            .blocks
            .iter()
            .filter(|block| !members.contains(&block.id))
        {
            if block.instructions.iter().any(|i| {
                i.memory
                    .as_ref()
                    .is_some_and(|m| memory_defs.contains(&m.input))
            }) {
                return false;
            }
            for edge in successors(&block.terminator) {
                if edge.memory_args.iter().any(|v| memory_defs.contains(v)) {
                    return false;
                }
                if members.contains(&edge.target) {
                    if block.id != base.preheader || edge.target != base.header {
                        return false;
                    }
                    entries += 1;
                }
            }
            if let KirTerminator::Return { memory, .. } = &block.terminator
                && memory.iter().any(|state| memory_defs.contains(&state.1))
            {
                return false;
            }
        }
        if entries != 1 || latches != 1 {
            return false;
        }
    }
    let mut used = BTreeSet::new();
    for (id, mapped) in &plan.accesses {
        let Some(base) = bases.get(&mapped.base) else {
            return false;
        };
        let Some((block, access)) = function.blocks.iter().find_map(|b| {
            b.instructions
                .iter()
                .find(|i| i.id == *id)
                .map(|i| (b.id, i))
        }) else {
            return false;
        };
        let Some(descriptor) = loops.loops.iter().find(|loop_| loop_.header == base.header) else {
            return false;
        };
        if !descriptor.blocks.contains(&block) {
            return false;
        }
        let Some(induction) = &descriptor.induction else {
            return false;
        };
        let place = match &access.kind {
            KirInstructionKind::Load { place } | KirInstructionKind::Store { place, .. } => {
                place.as_ref()
            }
            _ => return false,
        };
        let KirPlace::SliceIndex {
            slice,
            index,
            type_node,
            ..
        } = place
        else {
            return false;
        };
        if element_width(type_node) != Some(base.element_bytes)
            || !forwarded_from(function, *slice, base.slice)
            || type_of(function, *index) != Some(&MirType::Primitive(MirPrimitiveTypeName::U32))
        {
            return false;
        }
        let mut expression = *index;
        let mut copies = 0;
        let expression = loop {
            let Some((_, instruction)) = defining(function, expression) else {
                return false;
            };
            if let KirInstructionKind::Copy { value } = instruction.kind {
                copies += 1;
                if copies > 16 {
                    return false;
                }
                expression = value;
            } else {
                break instruction;
            }
        };
        let KirInstructionKind::Binary {
            op: MirBinaryOp::Add,
            left,
            right,
            semantics: KirArithmeticSemantics::Modular,
        } = expression.kind
        else {
            return false;
        };
        if expression.memory.is_some() || expression.effect.is_some() {
            return false;
        }
        let offset = if mapped.index == left {
            right
        } else if mapped.index == right {
            left
        } else {
            return false;
        };
        if !forwarded_from(function, mapped.index, induction.value)
            || !forwarded_from(function, offset, base.offset)
            || type_of(function, mapped.index)
                != Some(&MirType::Primitive(MirPrimitiveTypeName::U32))
        {
            return false;
        }
        used.insert(base.id);
    }
    used.len() == bases.len()
}

fn available_at_entry(
    function: &KirFunction,
    dominators: &crate::KirDominators,
    members: &BTreeSet<BlockId>,
    preheader: BlockId,
    value: ValueId,
) -> bool {
    if function.params.iter().any(|p| p.value == value) {
        return true;
    }
    function.blocks.iter().any(|block| {
        !members.contains(&block.id)
            && dominators.dominates(block.id, preheader)
            && (block.params.iter().any(|p| p.value == value)
                || block
                    .instructions
                    .iter()
                    .any(|i| i.results.iter().any(|r| r.value == value)))
    })
}

fn forwarded_from(function: &KirFunction, value: ValueId, root: ValueId) -> bool {
    let mut pending = vec![value];
    let mut seen = BTreeSet::new();
    let mut reaches_root = false;
    while let Some(value) = pending.pop() {
        if value == root {
            reaches_root = true;
            continue;
        }
        if !seen.insert(value) {
            continue;
        }
        if seen.len() > 2048 {
            return false;
        }
        if let Some((owner, index)) = function.blocks.iter().find_map(|b| {
            b.params
                .iter()
                .position(|p| p.value == value)
                .map(|i| (b.id, i))
        }) {
            let mut predecessors = 0;
            for block in &function.blocks {
                for edge in successors(&block.terminator) {
                    if edge.target == owner {
                        let Some(arg) = edge.args.get(index) else {
                            return false;
                        };
                        pending.push(*arg);
                        predecessors += 1;
                    }
                }
            }
            if predecessors == 0 {
                return false;
            }
        } else if let Some((_, instruction)) = defining(function, value) {
            if let KirInstructionKind::Copy { value } = instruction.kind {
                pending.push(value);
            } else {
                return false;
            }
        } else {
            return false;
        }
    }
    reaches_root
}

fn element_width(type_node: &MirType) -> Option<u32> {
    match type_node {
        MirType::Primitive(MirPrimitiveTypeName::I32 | MirPrimitiveTypeName::U32) => Some(4),
        MirType::Primitive(
            MirPrimitiveTypeName::I64 | MirPrimitiveTypeName::U64 | MirPrimitiveTypeName::F64,
        ) => Some(8),
        _ => None,
    }
}

fn type_of(function: &KirFunction, value: ValueId) -> Option<&MirType> {
    if let Some(parameter) = function.params.iter().find(|p| p.value == value) {
        return Some(&parameter.type_node);
    }
    for block in &function.blocks {
        if let Some(parameter) = block.params.iter().find(|p| p.value == value) {
            return parameter.type_node.as_scalar();
        }
        for instruction in &block.instructions {
            if let Some(result) = instruction.results.iter().find(|r| r.value == value) {
                return result.type_node.as_scalar();
            }
        }
    }
    None
}

fn defining(function: &KirFunction, value: ValueId) -> Option<(BlockId, &KirInstruction)> {
    for block in &function.blocks {
        for instruction in &block.instructions {
            if instruction.results.iter().any(|r| r.value == value) {
                return Some((block.id, instruction));
            }
        }
    }
    None
}

fn successors(terminator: &KirTerminator) -> Vec<&crate::KirEdge> {
    match terminator {
        KirTerminator::Jump { edge } => vec![edge],
        KirTerminator::Branch {
            then_edge,
            else_edge,
            ..
        } => vec![then_edge, else_edge],
        KirTerminator::Return { .. } => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source_function() -> KirFunction {
        let checked = crate::check(&crate::SourceFile::new(
            "row.ck",
            r#"
export unsafe fn row(input: slice<u32>, out: slice<u32>, offset: u32, n: u32) -> void contract {
  requires n <= out.len;
  effects read(input), readwrite(out);
} {
  let i: u32 = 0;
  while i < n {
    let value: u32 = input[offset + i];
    if value != 0 { out[i] = value; }
    i = i + 1;
  }
}
"#,
        ));
        assert!(checked.diagnostics.is_empty());
        let mir = crate::lower_to_mir(&checked.checked_program).unwrap();
        let module = crate::build_kir_module(
            &mir,
            crate::KirBuildConfig {
                consumer: crate::KirConsumer::WebAssembly,
                overflow_mode: crate::KirOverflowMode::Unchecked,
                bounds_mode: crate::KirBoundsMode::Unchecked,
                sanitizer_mode: crate::KirSanitizerMode::Disabled,
            },
        )
        .unwrap();
        let contracts = crate::import_contract_facts(&module, &checked.checked_program, 0).unwrap();
        let optimized =
            crate::run_kir_pass_pipeline(module, crate::KirOptimizationLevel::O3, Some(&contracts));
        assert!(optimized.errors.is_empty(), "{:?}", optimized.errors);
        optimized.artifact.unwrap().functions.remove(0)
    }

    #[test]
    fn factored_checker_reconstructs_source_and_rejects_false_address_maps() {
        let function = source_function();
        let plan = super::super::factored::checked_factored_addresses(&function);
        assert_eq!(plan.bases.len(), 1);
        assert!(validate(&function, &plan));
        let mut bad = plan.clone();
        bad.bases[0].element_bytes = 8;
        assert!(!validate(&function, &bad), "wrong byte scale");
        let mut bad = plan.clone();
        bad.bases[0].offset = bad.bases[0].slice;
        assert!(!validate(&function, &bad), "wrong source offset");
        let mut bad = plan.clone();
        bad.accesses.values_mut().next().unwrap().index = bad.bases[0].offset;
        assert!(!validate(&function, &bad), "wrong recurrence operand");
        let mut bad = plan.clone();
        bad.bases[0].preheader = bad.bases[0].header;
        assert!(
            !validate(&function, &bad),
            "initialization must precede the loop"
        );
        let mut bad = plan.clone();
        bad.accesses.clear();
        assert!(!validate(&function, &bad), "unreferenced base");
    }

    #[test]
    fn factored_checker_rejects_memory_state_escaping_without_exit_parameters() {
        let mut function = source_function();
        let plan = super::super::factored::checked_factored_addresses(&function);
        assert_eq!(plan.bases.len(), 1);
        let header = function
            .blocks
            .iter()
            .find(|block| block.id == plan.bases[0].header)
            .unwrap();
        let memory = header.memory_params[0].clone();
        let mut mutated = false;
        for block in &mut function.blocks {
            if let KirTerminator::Return { memory: states, .. } = &mut block.terminator {
                let state = states
                    .iter_mut()
                    .find(|(region, _)| *region == memory.region)
                    .unwrap();
                state.1 = memory.version;
                mutated = true;
                break;
            }
        }
        assert!(mutated);
        assert!(!validate(&function, &plan));
    }
}
