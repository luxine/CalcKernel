use std::collections::{BTreeMap, BTreeSet};

use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KirDominators {
    pub entry: Option<BlockId>,
    pub sets: BTreeMap<BlockId, BTreeSet<BlockId>>,
}

impl KirDominators {
    #[must_use]
    pub fn dominates(&self, dominator: BlockId, block: BlockId) -> bool {
        self.sets
            .get(&block)
            .is_some_and(|set| set.contains(&dominator))
    }
}

#[must_use]
pub fn compute_kir_dominators(function: &KirFunction) -> KirDominators {
    let Some(entry) = function.blocks.first().map(|block| block.id) else {
        return KirDominators {
            entry: None,
            sets: BTreeMap::new(),
        };
    };
    let all = function
        .blocks
        .iter()
        .map(|block| block.id)
        .collect::<BTreeSet<_>>();
    let mut predecessors = function
        .blocks
        .iter()
        .map(|block| (block.id, BTreeSet::new()))
        .collect::<BTreeMap<_, _>>();
    for block in &function.blocks {
        for successor in terminator_successors(&block.terminator) {
            if let Some(incoming) = predecessors.get_mut(&successor) {
                incoming.insert(block.id);
            }
        }
    }
    let mut sets = BTreeMap::new();
    for block in &function.blocks {
        sets.insert(
            block.id,
            if block.id == entry {
                BTreeSet::from([entry])
            } else if predecessors.get(&block.id).is_none_or(BTreeSet::is_empty) {
                BTreeSet::from([block.id])
            } else {
                all.clone()
            },
        );
    }
    loop {
        let mut changed = false;
        for block in function.blocks.iter().filter(|block| block.id != entry) {
            let incoming = predecessors.get(&block.id);
            let mut next = if incoming.is_none_or(BTreeSet::is_empty) {
                BTreeSet::new()
            } else {
                incoming
                    .into_iter()
                    .flatten()
                    .filter_map(|predecessor| sets.get(predecessor).cloned())
                    .reduce(|left, right| left.intersection(&right).copied().collect())
                    .unwrap_or_default()
            };
            next.insert(block.id);
            if sets.get(&block.id) != Some(&next) {
                sets.insert(block.id, next);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    KirDominators {
        entry: Some(entry),
        sets,
    }
}

#[must_use]
pub fn terminator_successors(terminator: &KirTerminator) -> Vec<BlockId> {
    match terminator {
        KirTerminator::Return { .. } => Vec::new(),
        KirTerminator::Jump { edge } => vec![edge.target],
        KirTerminator::Branch {
            then_edge,
            else_edge,
            ..
        } => vec![then_edge.target, else_edge.target],
    }
}
