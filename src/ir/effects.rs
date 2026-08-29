use std::collections::{BTreeMap, BTreeSet};

/// Memory root used by the shared source/KIR effect lattice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EffectTarget {
    Parameter(u32),
    All,
}

/// Access lattice ordered as None < Read/Write < ReadWrite.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MemoryEffect {
    None,
    Read,
    Write,
    ReadWrite,
}

impl MemoryEffect {
    #[must_use]
    pub const fn join(self, other: Self) -> Self {
        match (self, other) {
            (Self::None, effect) | (effect, Self::None) => effect,
            (Self::Read, Self::Read) => Self::Read,
            (Self::Write, Self::Write) => Self::Write,
            (Self::ReadWrite, _)
            | (_, Self::ReadWrite)
            | (Self::Read, Self::Write)
            | (Self::Write, Self::Read) => Self::ReadWrite,
        }
    }

    #[must_use]
    pub const fn allows(self, required: Self) -> bool {
        matches!(
            (self, required),
            (_, Self::None)
                | (Self::Read, Self::Read)
                | (Self::Write, Self::Write)
                | (Self::ReadWrite, _)
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EffectAccess {
    pub target: EffectTarget,
    pub effect: MemoryEffect,
}

/// Canonical complete function effect summary.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EffectSummary {
    memory: BTreeMap<EffectTarget, MemoryEffect>,
    pub runtime_effect: bool,
    pub may_fail: bool,
    pub unsafe_calls: bool,
    pub conservative: bool,
}

impl EffectSummary {
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            memory: BTreeMap::new(),
            runtime_effect: false,
            may_fail: false,
            unsafe_calls: false,
            conservative: false,
        }
    }

    #[must_use]
    pub fn from_access(access: EffectAccess) -> Self {
        let mut summary = Self::empty();
        summary.add_access(access);
        summary
    }

    #[must_use]
    pub fn full_conservative() -> Self {
        let mut summary = Self::from_access(EffectAccess {
            target: EffectTarget::All,
            effect: MemoryEffect::ReadWrite,
        });
        summary.runtime_effect = true;
        summary.may_fail = true;
        summary.unsafe_calls = true;
        summary.conservative = true;
        summary
    }

    pub fn add_access(&mut self, access: EffectAccess) {
        let current = self
            .memory
            .get(&access.target)
            .copied()
            .unwrap_or(MemoryEffect::None);
        let joined = current.join(access.effect);
        if joined == MemoryEffect::None {
            self.memory.remove(&access.target);
        } else {
            self.memory.insert(access.target, joined);
        }
    }

    pub fn join(&mut self, other: &Self) {
        for (target, effect) in &other.memory {
            self.add_access(EffectAccess {
                target: *target,
                effect: *effect,
            });
        }
        self.runtime_effect |= other.runtime_effect;
        self.may_fail |= other.may_fail;
        self.unsafe_calls |= other.unsafe_calls;
        self.conservative |= other.conservative;
    }

    #[must_use]
    pub fn effect(&self, target: EffectTarget) -> MemoryEffect {
        self.memory
            .get(&target)
            .copied()
            .unwrap_or(MemoryEffect::None)
            .join(
                self.memory
                    .get(&EffectTarget::All)
                    .copied()
                    .unwrap_or(MemoryEffect::None),
            )
    }

    pub fn accesses(&self) -> impl Iterator<Item = EffectAccess> + '_ {
        self.memory.iter().map(|(target, effect)| EffectAccess {
            target: *target,
            effect: *effect,
        })
    }
}

/// Call edge with callee-parameter to caller-root mapping. `None` means private storage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectCall {
    pub callee: String,
    pub arguments: Vec<Option<EffectTarget>>,
    pub is_unsafe: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectFunction {
    pub name: String,
    pub parameter_count: u32,
    pub direct: EffectSummary,
    pub calls: Vec<EffectCall>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EffectGraph {
    pub functions: Vec<EffectFunction>,
}

/// Adapter boundary shared by typed source and KIR call graphs.
pub trait EffectGraphAdapter {
    fn effect_graph(&self) -> EffectGraph;
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EffectSolveConfig {
    max_steps_override: Option<u32>,
}

impl EffectSolveConfig {
    #[must_use]
    pub const fn with_max_steps(max_steps: u32) -> Self {
        Self {
            max_steps_override: Some(max_steps),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectSolveResult {
    pub summaries: BTreeMap<String, EffectSummary>,
    pub sccs: Vec<Vec<String>>,
    pub steps: u32,
    pub max_steps: u32,
    pub exhausted: bool,
}

#[must_use]
pub fn solve_effects(
    adapter: &impl EffectGraphAdapter,
    config: EffectSolveConfig,
) -> EffectSolveResult {
    solve_effect_graph(&adapter.effect_graph(), config)
}

/// Deterministic bottom-up SCC solver. It never consults wall-clock time.
#[must_use]
pub fn solve_effect_graph(graph: &EffectGraph, config: EffectSolveConfig) -> EffectSolveResult {
    let functions = graph
        .functions
        .iter()
        .map(|function| (function.name.clone(), function))
        .collect::<BTreeMap<_, _>>();
    let units = graph.functions.iter().fold(0_usize, |count, function| {
        count + 1 + function.calls.len() + function.direct.accesses().count()
    });
    let units = u32::try_from(units).unwrap_or(u32::MAX);
    let max_steps = config
        .max_steps_override
        .unwrap_or_else(|| units.saturating_mul(32).saturating_add(64));
    let sccs = strongly_connected_components(&functions);
    let mut summaries = functions
        .iter()
        .map(|(name, function)| (name.clone(), function.direct.clone()))
        .collect::<BTreeMap<_, _>>();
    let mut solved = BTreeSet::new();
    let mut steps = 0_u32;
    let mut exhausted = false;
    while solved.len() < sccs.len() {
        let Some((component_index, component)) =
            sccs.iter().enumerate().find(|(index, component)| {
                !solved.contains(index)
                    && component.iter().all(|name| {
                        functions[name].calls.iter().all(|call| {
                            component.contains(&call.callee)
                                || !functions.contains_key(&call.callee)
                                || sccs
                                    .iter()
                                    .position(|candidate| candidate.contains(&call.callee))
                                    .is_some_and(|callee_index| solved.contains(&callee_index))
                        })
                    })
            })
        else {
            break;
        };
        let mut component_exhausted = false;
        loop {
            let mut changed = false;
            let previous = summaries.clone();
            for name in component {
                if steps >= max_steps {
                    component_exhausted = true;
                    break;
                }
                steps += 1;
                let function = functions[name];
                let mut next = function.direct.clone();
                for call in &function.calls {
                    next.unsafe_calls |= call.is_unsafe;
                    let callee = previous
                        .get(&call.callee)
                        .cloned()
                        .unwrap_or_else(EffectSummary::full_conservative);
                    next.join(&map_callee_summary(&callee, call));
                }
                if summaries.get(name) != Some(&next) {
                    summaries.insert(name.clone(), next);
                    changed = true;
                }
            }
            if component_exhausted || !changed {
                break;
            }
        }
        if component_exhausted {
            exhausted = true;
            for name in component {
                summaries.insert(name.clone(), EffectSummary::full_conservative());
            }
        }
        solved.insert(component_index);
    }
    EffectSolveResult {
        summaries,
        sccs,
        steps,
        max_steps,
        exhausted,
    }
}

fn map_callee_summary(summary: &EffectSummary, call: &EffectCall) -> EffectSummary {
    let mut mapped = EffectSummary::empty();
    mapped.runtime_effect = summary.runtime_effect;
    mapped.may_fail = summary.may_fail;
    mapped.unsafe_calls = summary.unsafe_calls || call.is_unsafe;
    mapped.conservative = summary.conservative;
    for access in summary.accesses() {
        let target = match access.target {
            EffectTarget::All => Some(EffectTarget::All),
            EffectTarget::Parameter(index) => match call.arguments.get(index as usize) {
                Some(target) => *target,
                None => Some(EffectTarget::All),
            },
        };
        if let Some(target) = target {
            mapped.add_access(EffectAccess {
                target,
                effect: access.effect,
            });
        }
    }
    mapped
}

fn strongly_connected_components(
    functions: &BTreeMap<String, &EffectFunction>,
) -> Vec<Vec<String>> {
    let reachability = functions
        .keys()
        .map(|name| (name.clone(), reachable_from(name, functions)))
        .collect::<BTreeMap<_, _>>();
    let mut assigned = BTreeSet::new();
    let mut components = Vec::new();
    for name in functions.keys() {
        if assigned.contains(name) {
            continue;
        }
        let component = functions
            .keys()
            .filter(|candidate| {
                !assigned.contains(*candidate)
                    && reachability[name].contains(*candidate)
                    && reachability[*candidate].contains(name)
            })
            .cloned()
            .collect::<Vec<_>>();
        for member in &component {
            assigned.insert(member.clone());
        }
        components.push(component);
    }
    components
}

fn reachable_from(start: &str, functions: &BTreeMap<String, &EffectFunction>) -> BTreeSet<String> {
    let mut reached = BTreeSet::new();
    let mut pending = vec![start.to_string()];
    while let Some(name) = pending.pop() {
        if !reached.insert(name.clone()) {
            continue;
        }
        if let Some(function) = functions.get(&name) {
            for callee in function
                .calls
                .iter()
                .map(|call| &call.callee)
                .filter(|callee| functions.contains_key(*callee))
                .rev()
            {
                pending.push(callee.clone());
            }
        }
    }
    reached
}
