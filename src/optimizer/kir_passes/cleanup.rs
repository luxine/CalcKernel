use crate::{KirModule, KirTerminator};

pub(crate) fn run_cleanup(module: &mut KirModule) -> bool {
    let mut changed = false;
    for function in &mut module.functions {
        let mut next_order = 0_u32;
        for block in &mut function.blocks {
            for instruction in &mut block.instructions {
                if let Some(effect) = &mut instruction.effect {
                    changed |= effect.order != next_order;
                    effect.order = next_order;
                    next_order = next_order.saturating_add(1);
                }
            }
            if let KirTerminator::Return { effect_order, .. } = &mut block.terminator {
                changed |= *effect_order != next_order;
                *effect_order = next_order;
                next_order = next_order.saturating_add(1);
            }
        }
    }
    changed
}
