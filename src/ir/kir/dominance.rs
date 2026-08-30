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
    match compute_dominators(function, |_| Ok::<(), std::convert::Infallible>(())) {
        Ok(result) => result,
        Err(never) => match never {},
    }
}

pub(crate) fn compute_kir_dominators_with_budget(
    function: &KirFunction,
    remaining: &mut u32,
) -> Option<KirDominators> {
    compute_dominators(function, |units| {
        let units = u32::try_from(units).map_err(|_| ())?;
        *remaining = remaining.checked_sub(units).ok_or(())?;
        Ok::<(), ()>(())
    })
    .ok()
}

fn compute_dominators<E>(
    function: &KirFunction,
    mut charge: impl FnMut(usize) -> Result<(), E>,
) -> Result<KirDominators, E> {
    let Some(entry) = function.blocks.first().map(|block| block.id) else {
        return Ok(KirDominators {
            entry: None,
            sets: BTreeMap::new(),
        });
    };
    let mut ids = function
        .blocks
        .iter()
        .map(|block| block.id)
        .collect::<Vec<_>>();
    ids.sort_unstable();
    let indices = ids
        .iter()
        .enumerate()
        .map(|(index, id)| (*id, index))
        .collect::<BTreeMap<_, _>>();
    let entry_index = indices[&entry];
    let mut predecessors = vec![Vec::new(); ids.len()];
    for block in &function.blocks {
        charge(1)?;
        let source = indices[&block.id];
        for successor in terminator_successors(&block.terminator) {
            if let Some(target) = indices.get(&successor) {
                predecessors[*target].push(source);
            }
        }
    }
    charge(ids.len().saturating_mul(ids.len()))?;
    let mut bits = vec![vec![true; ids.len()]; ids.len()];
    for index in 0..ids.len() {
        if index == entry_index || predecessors[index].is_empty() {
            bits[index].fill(false);
            bits[index][index] = true;
        }
    }
    loop {
        let mut changed = false;
        for index in 0..ids.len() {
            charge(ids.len())?;
            if index == entry_index {
                continue;
            }
            let mut next = if let Some(first) = predecessors[index].first() {
                bits[*first].clone()
            } else {
                vec![false; ids.len()]
            };
            for predecessor in predecessors[index].iter().skip(1) {
                charge(ids.len())?;
                for (candidate, dominates_predecessor) in
                    next.iter_mut().zip(bits[*predecessor].iter())
                {
                    *candidate &= *dominates_predecessor;
                }
            }
            next[index] = true;
            if bits[index] != next {
                bits[index] = next;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    charge(ids.len().saturating_mul(ids.len()))?;
    let sets = ids
        .iter()
        .enumerate()
        .map(|(block, id)| {
            let dominators = bits[block]
                .iter()
                .enumerate()
                .filter_map(|(candidate, present)| present.then_some(ids[candidate]))
                .collect();
            (*id, dominators)
        })
        .collect();
    Ok(KirDominators {
        entry: Some(entry),
        sets,
    })
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
