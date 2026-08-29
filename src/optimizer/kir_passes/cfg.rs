use std::collections::BTreeSet;

use crate::{KirInstructionKind, KirModule, KirTerminator, ValueId};

pub(crate) fn run_cfg_canonicalize(module: &mut KirModule) -> bool {
    let mut changed = false;
    for function in &mut module.functions {
        let constants = function
            .blocks
            .iter()
            .flat_map(|block| &block.instructions)
            .filter_map(|instruction| match instruction.kind {
                KirInstructionKind::ConstBool { value } => instruction
                    .results
                    .first()
                    .map(|result| (result.value, value)),
                _ => None,
            })
            .collect::<Vec<(ValueId, bool)>>();
        for block in &mut function.blocks {
            let replacement = match &block.terminator {
                KirTerminator::Branch {
                    condition,
                    then_edge,
                    else_edge,
                } => constants
                    .iter()
                    .find_map(|(value, constant)| (*value == *condition).then_some(*constant))
                    .map(|constant| KirTerminator::Jump {
                        edge: if constant {
                            then_edge.clone()
                        } else {
                            else_edge.clone()
                        },
                    }),
                _ => None,
            };
            if let Some(replacement) = replacement {
                block.terminator = replacement;
                changed = true;
            }
        }

        let Some(entry) = function.blocks.first().map(|block| block.id) else {
            continue;
        };
        let mut reachable = BTreeSet::from([entry]);
        loop {
            let before = reachable.len();
            for block in &function.blocks {
                if !reachable.contains(&block.id) {
                    continue;
                }
                match &block.terminator {
                    KirTerminator::Return { .. } => {}
                    KirTerminator::Jump { edge } => {
                        reachable.insert(edge.target);
                    }
                    KirTerminator::Branch {
                        then_edge,
                        else_edge,
                        ..
                    } => {
                        reachable.insert(then_edge.target);
                        reachable.insert(else_edge.target);
                    }
                }
            }
            if reachable.len() == before {
                break;
            }
        }
        let before = function.blocks.len();
        function
            .blocks
            .retain(|block| reachable.contains(&block.id));
        changed |= function.blocks.len() != before;
    }
    changed
}
