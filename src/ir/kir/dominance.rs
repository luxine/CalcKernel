use std::collections::{BTreeMap, BTreeSet};

use super::*;

const DOMINANCE_CACHE_CAPACITY: usize = 16;

#[derive(Clone, PartialEq, Eq)]
struct KirCfgIdentity {
    blocks: Vec<(BlockId, [Option<BlockId>; 2])>,
}

impl KirCfgIdentity {
    fn for_function(function: &KirFunction) -> Self {
        Self {
            blocks: function
                .blocks
                .iter()
                .map(|block| {
                    let successors = match &block.terminator {
                        KirTerminator::Return { .. } => [None, None],
                        KirTerminator::Jump { edge } => [Some(edge.target), None],
                        KirTerminator::Branch {
                            then_edge,
                            else_edge,
                            ..
                        } => [Some(then_edge.target), Some(else_edge.target)],
                    };
                    (block.id, successors)
                })
                .collect(),
        }
    }
}

#[derive(Clone)]
struct CachedDominators {
    identity: KirCfgIdentity,
    result: KirDominators,
    budget_units: u32,
}

thread_local! {
    static DOMINANCE_CACHE: std::cell::RefCell<std::collections::VecDeque<CachedDominators>> =
        const { std::cell::RefCell::new(std::collections::VecDeque::new()) };
    #[cfg(test)]
    static DOMINANCE_CACHE_MISSES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

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
    let identity = KirCfgIdentity::for_function(function);
    if let Some(cached) = cached_dominators(&identity) {
        return cached.result;
    }
    record_dominance_cache_miss();
    let mut budget_units = 0_u32;
    let result = match compute_dominators(function, |units| {
        budget_units = budget_units.saturating_add(u32::try_from(units).unwrap_or(u32::MAX));
        Ok::<(), std::convert::Infallible>(())
    }) {
        Ok(result) => result,
        Err(never) => match never {},
    };
    cache_dominators(CachedDominators {
        identity,
        result: result.clone(),
        budget_units,
    });
    result
}

#[cfg(test)]
fn clear_dominance_cache_for_test() {
    DOMINANCE_CACHE.with(|cache| cache.borrow_mut().clear());
    DOMINANCE_CACHE_MISSES.with(|misses| misses.set(0));
}

#[cfg(test)]
fn dominance_cache_misses_for_test() -> usize {
    DOMINANCE_CACHE_MISSES.with(std::cell::Cell::get)
}

pub(crate) fn compute_kir_dominators_with_budget(
    function: &KirFunction,
    remaining: &mut u32,
) -> Option<KirDominators> {
    let identity = KirCfgIdentity::for_function(function);
    if let Some(cached) = cached_dominators(&identity) {
        *remaining = remaining.checked_sub(cached.budget_units)?;
        return Some(cached.result);
    }
    record_dominance_cache_miss();
    let initial_remaining = *remaining;
    let result = compute_dominators(function, |units| {
        let units = u32::try_from(units).map_err(|_| ())?;
        *remaining = remaining.checked_sub(units).ok_or(())?;
        Ok::<(), ()>(())
    })
    .ok()?;
    cache_dominators(CachedDominators {
        identity,
        result: result.clone(),
        budget_units: initial_remaining.saturating_sub(*remaining),
    });
    Some(result)
}

fn cached_dominators(identity: &KirCfgIdentity) -> Option<CachedDominators> {
    DOMINANCE_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        let index = cache
            .iter()
            .position(|candidate| &candidate.identity == identity)?;
        let entry = cache.remove(index)?;
        let result = entry.clone();
        cache.push_front(entry);
        Some(result)
    })
}

fn cache_dominators(entry: CachedDominators) {
    DOMINANCE_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        cache.push_front(entry);
        cache.truncate(DOMINANCE_CACHE_CAPACITY);
    });
}

fn record_dominance_cache_miss() {
    #[cfg(test)]
    DOMINANCE_CACHE_MISSES.with(|misses| misses.set(misses.get().saturating_add(1)));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dominance_cache_should_follow_cfg_identity_not_instruction_identity() {
        clear_dominance_cache_for_test();
        let mut function = KirFunction {
            id: FunctionId::from_index(0),
            name: "cached".to_string(),
            exported: false,
            params: Vec::new(),
            return_type: crate::MirType::Void,
            regions: Vec::new(),
            initial_memory: Vec::new(),
            vector_regions: Vec::new(),
            blocks: vec![KirBlock {
                id: BlockId::from_index(0),
                label: "entry".to_string(),
                params: Vec::new(),
                memory_params: Vec::new(),
                instructions: Vec::new(),
                terminator: KirTerminator::Return {
                    value: None,
                    memory: Vec::new(),
                    effect_order: 0,
                },
            }],
        };
        let _ = compute_kir_dominators(&function);
        function.name.push_str("-instruction-only-change");
        let _ = compute_kir_dominators(&function);
        assert_eq!(dominance_cache_misses_for_test(), 1);

        function.blocks[0].terminator = KirTerminator::Jump {
            edge: KirEdge {
                target: BlockId::from_index(0),
                args: Vec::new(),
                memory_args: Vec::new(),
            },
        };
        let _ = compute_kir_dominators(&function);
        assert_eq!(dominance_cache_misses_for_test(), 2);
    }
}
