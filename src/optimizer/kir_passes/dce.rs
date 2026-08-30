use std::collections::BTreeSet;

use crate::{
    InstructionId, KirFunction, KirInstruction, KirInstructionKind, KirMemoryRegionOrigin,
    KirModule, KirPlace, KirTerminator, MemoryRegionId, ValueId,
};

pub(crate) fn run_dead_code_elimination(
    module: &mut KirModule,
    protected: &BTreeSet<InstructionId>,
) -> bool {
    let mut changed = false;
    loop {
        let used = collect_used_values(module);
        let mut removed = false;
        for function in &mut module.functions {
            for block in &mut function.blocks {
                let before = block.instructions.len();
                block
                    .instructions
                    .retain(|instruction| !is_dead_pure_instruction(instruction, &used, protected));
                removed |= block.instructions.len() != before;
            }
        }
        changed |= removed;
        if !removed {
            break;
        }
    }
    for function in &mut module.functions {
        changed |= remove_dead_descriptor_regions(function);
    }
    changed
}

fn remove_dead_descriptor_regions(function: &mut KirFunction) -> bool {
    let defined = function
        .params
        .iter()
        .map(|param| param.value)
        .chain(function.blocks.iter().flat_map(|block| {
            block.params.iter().map(|param| param.value).chain(
                block
                    .instructions
                    .iter()
                    .flat_map(|instruction| instruction.results.iter().map(|result| result.value)),
            )
        }))
        .collect::<BTreeSet<_>>();
    let mut direct = BTreeSet::new();
    direct.extend(function.initial_memory.iter().map(|memory| memory.region));
    for block in &function.blocks {
        direct.extend(block.memory_params.iter().map(|param| param.region));
        if let KirTerminator::Return { memory, .. } = &block.terminator {
            direct.extend(memory.iter().map(|(region, _)| *region));
        }
        for instruction in &block.instructions {
            if let Some(memory) = &instruction.memory {
                direct.insert(memory.region);
            }
            match &instruction.kind {
                KirInstructionKind::Address { place }
                | KirInstructionKind::Load { place }
                | KirInstructionKind::Store { place, .. } => {
                    collect_place_regions(place, &mut direct)
                }
                _ => {}
            }
        }
    }
    let mut changed = false;
    loop {
        let mut referenced = direct.clone();
        for region in &function.regions {
            referenced.extend(region.parent);
            referenced.insert(region.partition);
        }
        let before = function.regions.len();
        function.regions.retain(|region| {
            referenced.contains(&region.id)
                || match region.origin {
                    KirMemoryRegionOrigin::RawSlice(value)
                    | KirMemoryRegionOrigin::Subslice(value) => defined.contains(&value),
                    KirMemoryRegionOrigin::Conservative | KirMemoryRegionOrigin::Parameter(_) => {
                        true
                    }
                }
        });
        if function.regions.len() == before {
            break;
        }
        changed = true;
    }
    changed
}

fn collect_place_regions(place: &KirPlace, regions: &mut BTreeSet<MemoryRegionId>) {
    match place {
        KirPlace::Value { region, .. }
        | KirPlace::Deref { region, .. }
        | KirPlace::SliceIndex { region, .. } => {
            regions.insert(*region);
        }
        KirPlace::Index { base, region, .. } | KirPlace::Field { base, region, .. } => {
            regions.insert(*region);
            collect_place_regions(base, regions);
        }
    }
}

fn is_dead_pure_instruction(
    instruction: &KirInstruction,
    used: &BTreeSet<ValueId>,
    protected: &BTreeSet<InstructionId>,
) -> bool {
    !protected.contains(&instruction.id)
        && instruction.effect.is_none()
        && instruction.memory.is_none()
        && is_pure(&instruction.kind)
        && !instruction.results.is_empty()
        && instruction
            .results
            .iter()
            .all(|result| !used.contains(&result.value))
}

fn is_pure(kind: &KirInstructionKind) -> bool {
    !matches!(
        kind,
        KirInstructionKind::Guard { .. }
            | KirInstructionKind::Load { .. }
            | KirInstructionKind::Store { .. }
            | KirInstructionKind::Call { .. }
            | KirInstructionKind::RuntimeCall { .. }
    )
}

fn collect_used_values(module: &KirModule) -> BTreeSet<ValueId> {
    let mut used = BTreeSet::new();
    for function in &module.functions {
        for block in &function.blocks {
            for instruction in &block.instructions {
                used.extend(instruction_uses(instruction));
            }
            match &block.terminator {
                KirTerminator::Return { value, .. } => used.extend(value),
                KirTerminator::Jump { edge } => used.extend(&edge.args),
                KirTerminator::Branch {
                    condition,
                    then_edge,
                    else_edge,
                } => {
                    used.insert(*condition);
                    used.extend(&then_edge.args);
                    used.extend(&else_edge.args);
                }
            }
        }
    }
    used
}

pub(super) fn instruction_uses(instruction: &KirInstruction) -> Vec<ValueId> {
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
