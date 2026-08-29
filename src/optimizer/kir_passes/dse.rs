use crate::{KirInstructionKind, KirModule};

pub(crate) fn run_dead_store_elimination(module: &mut KirModule) -> u32 {
    let mut rewrites = 0_u32;
    for function in &mut module.functions {
        for block in &mut function.blocks {
            let mut index = 0_usize;
            while index < block.instructions.len() {
                let Some((first_input, first_output, first_region, first_place)) =
                    store_signature(&block.instructions[index])
                else {
                    index += 1;
                    continue;
                };
                let next_effect = block.instructions[index + 1..]
                    .iter()
                    .position(|instruction| instruction.effect.is_some())
                    .map(|offset| index + 1 + offset);
                let Some(next_index) = next_effect else {
                    index += 1;
                    continue;
                };
                let Some((_, _, next_region, next_place)) =
                    store_signature(&block.instructions[next_index])
                else {
                    index += 1;
                    continue;
                };
                let next_memory = block.instructions[next_index]
                    .memory
                    .as_mut()
                    .expect("store memory");
                if next_region == first_region
                    && next_place == first_place
                    && next_memory.input == first_output
                {
                    next_memory.input = first_input;
                    block.instructions.remove(index);
                    rewrites = rewrites.saturating_add(1);
                } else {
                    index += 1;
                }
            }
        }
    }
    rewrites
}

fn store_signature(
    instruction: &crate::KirInstruction,
) -> Option<(
    crate::MemoryVersionId,
    crate::MemoryVersionId,
    crate::MemoryRegionId,
    crate::KirPlace,
)> {
    let KirInstructionKind::Store { place, .. } = &instruction.kind else {
        return None;
    };
    let memory = instruction.memory.as_ref()?;
    Some((
        memory.input,
        memory.output?,
        memory.region,
        place.as_ref().clone(),
    ))
}
