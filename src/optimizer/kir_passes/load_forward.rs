use crate::{KirInstructionKind, KirModule};

use super::rewrite::replace_value_uses;

pub(crate) fn run_load_forwarding(module: &mut KirModule) -> u32 {
    let mut rewrites = 0_u32;
    for function in &mut module.functions {
        let mut candidates = Vec::new();
        for block in &function.blocks {
            for (index, instruction) in block.instructions.iter().enumerate() {
                let KirInstructionKind::Load { place } = &instruction.kind else {
                    continue;
                };
                let (Some(memory), Some(result)) =
                    (&instruction.memory, instruction.results.first())
                else {
                    continue;
                };
                let replacement =
                    block.instructions[..index].iter().rev().find_map(|source| {
                        match (&source.kind, &source.memory) {
                            (
                                KirInstructionKind::Load {
                                    place: source_place,
                                },
                                Some(source_memory),
                            ) if source_place == place
                                && source_memory.region == memory.region
                                && source_memory.input == memory.input =>
                            {
                                source.results.first().map(|result| result.value)
                            }
                            (
                                KirInstructionKind::Store {
                                    place: source_place,
                                    value,
                                },
                                Some(source_memory),
                            ) if source_place == place
                                && source_memory.region == memory.region
                                && source_memory.output == Some(memory.input) =>
                            {
                                Some(*value)
                            }
                            _ => None,
                        }
                    });
                if let Some(replacement) = replacement {
                    candidates.push((instruction.id, result.value, replacement));
                }
            }
        }
        for (_, old, new) in &candidates {
            replace_value_uses(function, *old, *new);
        }
        for block in &mut function.blocks {
            block.instructions.retain(|instruction| {
                !candidates
                    .iter()
                    .any(|(removed, _, _)| *removed == instruction.id)
            });
        }
        rewrites = rewrites.saturating_add(u32::try_from(candidates.len()).unwrap_or(u32::MAX));
    }
    rewrites
}
