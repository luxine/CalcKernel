use std::collections::{HashMap, HashSet};

use crate::{
    MirBlock, MirFunction, MirInstruction, MirInstructionEffect, MirLocal, MirModule,
    MirTerminator, MirValue, instruction_effect, optimizer_artifact_roots,
    reachable_function_names,
};

use super::super::{analysis::*, pipeline::*};

#[derive(Debug, Clone, PartialEq, Eq)]
struct InlineCandidate {
    func: MirFunction,
    block: MirBlock,
    return_value: MirValue,
}

struct InlineState {
    call_index: usize,
    existing_names: HashSet<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct InlineRewriteMaps {
    params: HashMap<String, MirValue>,
    locals: HashMap<String, String>,
    temps: HashMap<String, String>,
}

pub(in crate::optimizer) fn run_inline_small_functions(
    module: &mut MirModule,
    context: &MirPassContext,
) -> MirPassResult {
    if context.opt_level < 2 {
        return MirPassResult {
            changed: false,
            diagnostics: Vec::new(),
        };
    }

    let threshold = if context.opt_level == 2 { 8 } else { 25 };
    let cyclic_functions = find_cyclic_functions(&module.functions);
    let candidates = collect_inline_candidates(&module.functions, &cyclic_functions, threshold);
    let mut changed = false;

    for function in &mut module.functions {
        changed |= inline_calls_in_function(function, &candidates);
    }

    if changed {
        changed |= remove_unreferenced_internal_functions(module);
    }

    MirPassResult {
        changed,
        diagnostics: Vec::new(),
    }
}

fn collect_inline_candidates(
    functions: &[MirFunction],
    cyclic_functions: &HashSet<String>,
    threshold: usize,
) -> HashMap<String, InlineCandidate> {
    let mut candidates = HashMap::new();
    for function in functions {
        if function.exported
            || cyclic_functions.contains(&function.name)
            || function.blocks.len() != 1
            || contains_slice_type(&function.return_type)
            || function
                .params
                .iter()
                .any(|param| contains_slice_type(&param.type_node))
        {
            continue;
        }
        let block = &function.blocks[0];
        let MirTerminator::Return { value } = &block.terminator else {
            continue;
        };
        let Some(value) = value else {
            continue;
        };
        if block.instructions.len() > threshold
            || !block.instructions.iter().all(is_inlineable_instruction)
        {
            continue;
        }
        candidates.insert(
            function.name.clone(),
            InlineCandidate {
                func: function.clone(),
                block: block.clone(),
                return_value: value.clone(),
            },
        );
    }
    candidates
}

fn is_inlineable_instruction(instruction: &MirInstruction) -> bool {
    matches!(
        instruction_effect(instruction),
        MirInstructionEffect::Pure | MirInstructionEffect::ObservableOutput
    ) && matches!(
        instruction,
        MirInstruction::ConstInt { .. }
            | MirInstruction::ConstFloat { .. }
            | MirInstruction::ConstBool { .. }
            | MirInstruction::Move { .. }
            | MirInstruction::Binary { .. }
            | MirInstruction::Unary { .. }
            | MirInstruction::Compare { .. }
            | MirInstruction::RuntimeCall { .. }
    )
}

fn inline_calls_in_function(
    function: &mut MirFunction,
    candidates: &HashMap<String, InlineCandidate>,
) -> bool {
    let mut state = InlineState {
        call_index: 0,
        existing_names: collect_function_value_names(function),
    };
    let mut changed = false;
    let mut new_locals = Vec::new();

    for block in &mut function.blocks {
        let mut instructions = Vec::with_capacity(block.instructions.len());
        for instruction in std::mem::take(&mut block.instructions) {
            let MirInstruction::Call {
                target,
                function_name,
                args,
            } = &instruction
            else {
                instructions.push(instruction);
                continue;
            };

            let Some(candidate) = candidates.get(function_name) else {
                instructions.push(instruction);
                continue;
            };
            if candidate.func.name == function.name {
                instructions.push(instruction);
                continue;
            }
            let Some(target) = target else {
                instructions.push(instruction);
                continue;
            };

            instructions.extend(instantiate_inline_candidate(
                candidate,
                target,
                args,
                &mut new_locals,
                &mut state,
            ));
            changed = true;
        }
        block.instructions = instructions;
    }
    function.locals.extend(new_locals);

    changed
}

fn instantiate_inline_candidate(
    candidate: &InlineCandidate,
    call_target: &MirValue,
    call_args: &[MirValue],
    new_locals: &mut Vec<MirLocal>,
    state: &mut InlineState,
) -> Vec<MirInstruction> {
    let prefix = format!("inl{}", state.call_index);
    state.call_index += 1;

    let mut maps = InlineRewriteMaps::default();
    for (param, arg) in candidate.func.params.iter().zip(call_args) {
        maps.params.insert(param.name.clone(), arg.clone());
    }

    for local in &candidate.func.locals {
        let name = unique_inline_name(
            &format!("{prefix}_{}", local.name),
            &mut state.existing_names,
        );
        maps.locals.insert(local.name.clone(), name.clone());
        new_locals.push(MirLocal {
            name,
            type_node: local.type_node.clone(),
        });
    }

    let mut instructions = candidate
        .block
        .instructions
        .iter()
        .map(|instruction| {
            clone_inline_instruction(instruction, &mut maps, &prefix, &mut state.existing_names)
        })
        .collect::<Vec<_>>();
    instructions.push(MirInstruction::Move {
        target: call_target.clone(),
        value: rewrite_inline_value(&candidate.return_value, &maps),
    });
    instructions
}

fn clone_inline_instruction(
    instruction: &MirInstruction,
    maps: &mut InlineRewriteMaps,
    prefix: &str,
    existing_names: &mut HashSet<String>,
) -> MirInstruction {
    match instruction {
        MirInstruction::ConstInt { target, value } => MirInstruction::ConstInt {
            target: rewrite_inline_target(target, maps, prefix, existing_names),
            value: value.clone(),
        },
        MirInstruction::ConstFloat { target, value } => MirInstruction::ConstFloat {
            target: rewrite_inline_target(target, maps, prefix, existing_names),
            value: value.clone(),
        },
        MirInstruction::ConstBool { target, value } => MirInstruction::ConstBool {
            target: rewrite_inline_target(target, maps, prefix, existing_names),
            value: *value,
        },
        MirInstruction::Move { target, value } => MirInstruction::Move {
            target: rewrite_inline_target(target, maps, prefix, existing_names),
            value: rewrite_inline_value(value, maps),
        },
        MirInstruction::Binary {
            target,
            op,
            left,
            right,
        } => MirInstruction::Binary {
            target: rewrite_inline_target(target, maps, prefix, existing_names),
            op: *op,
            left: rewrite_inline_value(left, maps),
            right: rewrite_inline_value(right, maps),
        },
        MirInstruction::Unary {
            target,
            op,
            operand,
        } => MirInstruction::Unary {
            target: rewrite_inline_target(target, maps, prefix, existing_names),
            op: *op,
            operand: rewrite_inline_value(operand, maps),
        },
        MirInstruction::Compare {
            target,
            op,
            left,
            right,
        } => MirInstruction::Compare {
            target: rewrite_inline_target(target, maps, prefix, existing_names),
            op: *op,
            left: rewrite_inline_value(left, maps),
            right: rewrite_inline_value(right, maps),
        },
        MirInstruction::RuntimeCall { intrinsic, args } => MirInstruction::RuntimeCall {
            intrinsic: *intrinsic,
            args: args
                .iter()
                .map(|arg| rewrite_inline_value(arg, maps))
                .collect(),
        },
        MirInstruction::Cast { .. }
        | MirInstruction::Address { .. }
        | MirInstruction::Load { .. }
        | MirInstruction::Store { .. }
        | MirInstruction::MakeSlice { .. }
        | MirInstruction::SliceData { .. }
        | MirInstruction::SliceLen { .. }
        | MirInstruction::Subslice { .. }
        | MirInstruction::Call { .. } => {
            unreachable!("candidate instruction must be inlineable")
        }
    }
}

fn rewrite_inline_target(
    target: &MirValue,
    maps: &mut InlineRewriteMaps,
    prefix: &str,
    existing_names: &mut HashSet<String>,
) -> MirValue {
    match target {
        MirValue::Temp { name, type_node } => {
            let name = maps
                .temps
                .entry(name.clone())
                .or_insert_with(|| unique_inline_name(&format!("{prefix}_{name}"), existing_names));
            MirValue::Temp {
                name: name.clone(),
                type_node: type_node.clone(),
            }
        }
        MirValue::Local { name, type_node } => maps.locals.get(name).map_or_else(
            || target.clone(),
            |name| MirValue::Local {
                name: name.clone(),
                type_node: type_node.clone(),
            },
        ),
        MirValue::Param { .. }
        | MirValue::ConstInt { .. }
        | MirValue::ConstFloat { .. }
        | MirValue::ConstBool { .. } => target.clone(),
    }
}

fn rewrite_inline_value(value: &MirValue, maps: &InlineRewriteMaps) -> MirValue {
    match value {
        MirValue::Param { name, .. } => maps
            .params
            .get(name)
            .cloned()
            .unwrap_or_else(|| value.clone()),
        MirValue::Local { name, type_node } => maps.locals.get(name).map_or_else(
            || value.clone(),
            |name| MirValue::Local {
                name: name.clone(),
                type_node: type_node.clone(),
            },
        ),
        MirValue::Temp { name, type_node } => maps.temps.get(name).map_or_else(
            || value.clone(),
            |name| MirValue::Temp {
                name: name.clone(),
                type_node: type_node.clone(),
            },
        ),
        MirValue::ConstInt { .. } | MirValue::ConstFloat { .. } | MirValue::ConstBool { .. } => {
            value.clone()
        }
    }
}

fn collect_function_value_names(function: &MirFunction) -> HashSet<String> {
    let mut names = HashSet::new();
    for param in &function.params {
        names.insert(param.name.clone());
    }
    for local in &function.locals {
        names.insert(local.name.clone());
    }
    for block in &function.blocks {
        for instruction in &block.instructions {
            if let Some(target) = instruction_target(instruction) {
                collect_value_name(target, &mut names);
            }
        }
    }
    names
}

fn collect_value_name(value: &MirValue, names: &mut HashSet<String>) {
    match value {
        MirValue::Param { name, .. }
        | MirValue::Local { name, .. }
        | MirValue::Temp { name, .. } => {
            names.insert(name.clone());
        }
        MirValue::ConstInt { .. } | MirValue::ConstFloat { .. } | MirValue::ConstBool { .. } => {}
    }
}

fn unique_inline_name(base: &str, existing_names: &mut HashSet<String>) -> String {
    if existing_names.insert(base.to_string()) {
        return base.to_string();
    }

    let mut suffix = 1;
    loop {
        let name = format!("{base}_{suffix}");
        if existing_names.insert(name.clone()) {
            return name;
        }
        suffix += 1;
    }
}

fn remove_unreferenced_internal_functions(module: &mut MirModule) -> bool {
    let roots = optimizer_artifact_roots(module);
    let reachable = reachable_function_names(module, &roots)
        .into_iter()
        .collect::<HashSet<_>>();
    let before = module.functions.len();
    module
        .functions
        .retain(|function| reachable.contains(&function.name));
    module.functions.len() != before
}

fn find_cyclic_functions(functions: &[MirFunction]) -> HashSet<String> {
    let graph = functions
        .iter()
        .map(|function| (function.name.clone(), collect_callees(function)))
        .collect::<HashMap<_, _>>();
    let mut cyclic = HashSet::new();
    let mut visited = HashSet::new();
    let mut active = HashSet::new();
    let mut stack = Vec::new();

    for function in functions {
        visit_call_graph(
            &function.name,
            &graph,
            &mut visited,
            &mut active,
            &mut stack,
            &mut cyclic,
        );
    }
    cyclic
}

fn visit_call_graph(
    name: &str,
    graph: &HashMap<String, HashSet<String>>,
    visited: &mut HashSet<String>,
    active: &mut HashSet<String>,
    stack: &mut Vec<String>,
    cyclic: &mut HashSet<String>,
) {
    if active.contains(name) {
        if let Some(cycle_start) = stack.iter().position(|entry| entry == name) {
            cyclic.extend(stack[cycle_start..].iter().cloned());
        }
        cyclic.insert(name.to_string());
        return;
    }
    if !visited.insert(name.to_string()) {
        return;
    }

    active.insert(name.to_string());
    stack.push(name.to_string());

    if let Some(callees) = graph.get(name) {
        for callee in callees {
            if graph.contains_key(callee) {
                visit_call_graph(callee, graph, visited, active, stack, cyclic);
            }
        }
    }

    stack.pop();
    active.remove(name);
}

fn collect_callees(function: &MirFunction) -> HashSet<String> {
    let mut callees = HashSet::new();
    for block in &function.blocks {
        for instruction in &block.instructions {
            if let MirInstruction::Call { function_name, .. } = instruction {
                callees.insert(function_name.clone());
            }
        }
    }
    callees
}
