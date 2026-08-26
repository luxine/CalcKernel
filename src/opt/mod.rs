use std::collections::{HashMap, HashSet};

use crate::{
    MirBinaryOp, MirBlock, MirCompareOp, MirFunction, MirInstruction, MirLocal, MirModule,
    MirPlace, MirPrimitiveTypeName, MirTerminator, MirType, MirUnaryOp, MirValidationError,
    MirValue, validate_mir_module,
};

pub type OptimizationLevel = u8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MirPassTargetBackend {
    Mir,
    C,
    Wasm,
    Llvm,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MirPassOverflowMode {
    Unchecked,
    Checked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MirPassBoundsMode {
    Unchecked,
    Checked,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MirPassDebugFlags {
    pub print_pass_pipeline: bool,
    pub print_mir_before_opt: bool,
    pub print_mir_after_opt: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirPassContext {
    pub opt_level: OptimizationLevel,
    pub overflow_mode: MirPassOverflowMode,
    pub bounds_mode: MirPassBoundsMode,
    pub target_backend: MirPassTargetBackend,
    pub debug: MirPassDebugFlags,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirPassResult {
    pub changed: bool,
    pub diagnostics: Vec<String>,
}

#[derive(Debug, Clone, Copy)]
pub struct MirPass {
    pub name: &'static str,
    run: fn(&mut MirModule, &MirPassContext) -> MirPassResult,
}

impl MirPass {
    fn new(name: &'static str, run: fn(&mut MirModule, &MirPassContext) -> MirPassResult) -> Self {
        Self { name, run }
    }

    fn run(self, module: &mut MirModule, context: &MirPassContext) -> MirPassResult {
        (self.run)(module, context)
    }
}

#[derive(Debug, Clone)]
pub struct MirOptimizationPipeline {
    pub opt_level: OptimizationLevel,
    pub passes: Vec<MirPass>,
    pub validate_after_each_pass: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirPassRecord {
    pub name: String,
    pub changed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirPassManagerResult {
    pub module: MirModule,
    pub changed: bool,
    pub records: Vec<MirPassRecord>,
    pub diagnostics: Vec<String>,
    pub validation_errors: Vec<MirValidationError>,
}

pub fn identity_pass() -> MirPass {
    MirPass::new("identity", no_op_pass)
}

pub fn constant_folding_pass() -> MirPass {
    MirPass::new("constant-folding", run_constant_folding)
}

pub fn copy_propagation_pass() -> MirPass {
    MirPass::new("copy-propagation", run_copy_propagation)
}

pub fn dead_code_elimination_pass() -> MirPass {
    MirPass::new("dead-code-elimination", run_dead_code_elimination)
}

pub fn cfg_simplify_pass() -> MirPass {
    MirPass::new("cfg-simplify", run_cfg_simplify)
}

fn inline_small_functions_pass() -> MirPass {
    MirPass::new("inline-small-functions", run_inline_small_functions)
}

fn local_cse_pass() -> MirPass {
    MirPass::new("local-cse", run_local_cse)
}

fn address_cse_pass() -> MirPass {
    MirPass::new("address-cse", run_address_cse)
}

fn loop_analysis_pass() -> MirPass {
    MirPass::new("loop-analysis", no_op_pass)
}

fn loop_invariant_code_motion_pass() -> MirPass {
    MirPass::new("loop-invariant-code-motion", run_loop_invariant_code_motion)
}

fn induction_simplify_pass() -> MirPass {
    MirPass::new("induction-simplify", no_op_pass)
}

#[must_use]
pub fn build_mir_optimization_pipeline(opt_level: OptimizationLevel) -> MirOptimizationPipeline {
    let passes = match opt_level {
        0 => Vec::new(),
        1 => vec![
            constant_folding_pass(),
            copy_propagation_pass(),
            dead_code_elimination_pass(),
            cfg_simplify_pass(),
        ],
        2 => vec![
            constant_folding_pass(),
            copy_propagation_pass(),
            inline_small_functions_pass(),
            constant_folding_pass(),
            copy_propagation_pass(),
            local_cse_pass(),
            copy_propagation_pass(),
            address_cse_pass(),
            dead_code_elimination_pass(),
            cfg_simplify_pass(),
            dead_code_elimination_pass(),
        ],
        _ => vec![
            constant_folding_pass(),
            copy_propagation_pass(),
            inline_small_functions_pass(),
            constant_folding_pass(),
            copy_propagation_pass(),
            loop_analysis_pass(),
            loop_invariant_code_motion_pass(),
            induction_simplify_pass(),
            constant_folding_pass(),
            copy_propagation_pass(),
            local_cse_pass(),
            copy_propagation_pass(),
            address_cse_pass(),
            dead_code_elimination_pass(),
            cfg_simplify_pass(),
            dead_code_elimination_pass(),
        ],
    };

    MirOptimizationPipeline {
        opt_level,
        passes,
        validate_after_each_pass: true,
    }
}

#[must_use]
pub fn print_mir_pass_pipeline(pipeline: &MirOptimizationPipeline) -> String {
    if pipeline.passes.is_empty() {
        return format!("O{}: <validator only>", pipeline.opt_level);
    }
    format!(
        "O{}: {}",
        pipeline.opt_level,
        pipeline
            .passes
            .iter()
            .map(|pass| pass.name)
            .collect::<Vec<_>>()
            .join(" -> ")
    )
}

pub fn run_mir_pass_pipeline(
    mut module: MirModule,
    pipeline: &MirOptimizationPipeline,
    context: &MirPassContext,
) -> MirPassManagerResult {
    let mut records = Vec::new();
    let mut diagnostics = Vec::new();
    let mut validation_errors = Vec::new();
    let mut changed = false;

    for pass in &pipeline.passes {
        let result = pass.run(&mut module, context);
        records.push(MirPassRecord {
            name: pass.name.to_string(),
            changed: result.changed,
        });
        changed |= result.changed;
        diagnostics.extend(result.diagnostics);

        if pipeline.validate_after_each_pass {
            validation_errors.extend(validate_mir_module(&module).errors);
        }
    }

    if pipeline.passes.is_empty() || !pipeline.validate_after_each_pass {
        validation_errors.extend(validate_mir_module(&module).errors);
    }

    MirPassManagerResult {
        module,
        changed,
        records,
        diagnostics,
        validation_errors,
    }
}

fn no_op_pass(_module: &mut MirModule, _context: &MirPassContext) -> MirPassResult {
    MirPassResult {
        changed: false,
        diagnostics: Vec::new(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct InlineCandidate {
    func: MirFunction,
    block: MirBlock,
    return_value: MirValue,
}

#[derive(Debug, Clone, PartialEq, Eq)]
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

fn run_inline_small_functions(module: &mut MirModule, context: &MirPassContext) -> MirPassResult {
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
        instruction,
        MirInstruction::ConstInt { .. }
            | MirInstruction::ConstFloat { .. }
            | MirInstruction::ConstBool { .. }
            | MirInstruction::Move { .. }
            | MirInstruction::Binary { .. }
            | MirInstruction::Unary { .. }
            | MirInstruction::Compare { .. }
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
        MirInstruction::Cast { .. }
        | MirInstruction::Address { .. }
        | MirInstruction::Load { .. }
        | MirInstruction::Store { .. }
        | MirInstruction::MakeSlice { .. }
        | MirInstruction::SliceData { .. }
        | MirInstruction::SliceLen { .. }
        | MirInstruction::Subslice { .. }
        | MirInstruction::Call { .. } => unreachable!("candidate instruction must be inlineable"),
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
    let mut referenced = HashSet::new();
    for function in &module.functions {
        for block in &function.blocks {
            for instruction in &block.instructions {
                if let MirInstruction::Call { function_name, .. } = instruction {
                    referenced.insert(function_name.clone());
                }
            }
        }
    }

    let before = module.functions.len();
    module
        .functions
        .retain(|function| function.exported || referenced.contains(&function.name));
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct LoopBackEdge {
    from: String,
    to: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NaturalLoop {
    header: String,
    back_edge: LoopBackEdge,
    blocks: HashSet<String>,
    preheader: String,
    exit_blocks: Vec<String>,
}

fn run_loop_invariant_code_motion(
    module: &mut MirModule,
    context: &MirPassContext,
) -> MirPassResult {
    if context.opt_level < 3
        || context.overflow_mode == MirPassOverflowMode::Checked
        || context.bounds_mode == MirPassBoundsMode::Checked
    {
        return MirPassResult {
            changed: false,
            diagnostics: Vec::new(),
        };
    }

    let mut changed = false;
    for function in &mut module.functions {
        changed |= hoist_loop_invariants(function);
    }

    MirPassResult {
        changed,
        diagnostics: Vec::new(),
    }
}

fn hoist_loop_invariants(function: &mut MirFunction) -> bool {
    let mut changed = false;
    for natural_loop in analyze_natural_loops(function) {
        changed |= hoist_in_loop(function, &natural_loop);
    }
    changed
}

fn hoist_in_loop(function: &mut MirFunction, natural_loop: &NaturalLoop) -> bool {
    let loop_defined_temps = collect_loop_defined_temps(function, &natural_loop.blocks);
    let loop_assigned_locals = collect_loop_assigned_locals(function, &natural_loop.blocks);
    let mut hoisted_temps = HashSet::new();
    let mut hoisted_instructions = Vec::new();
    let mut changed = false;

    for block in &mut function.blocks {
        if !natural_loop.blocks.contains(&block.label) {
            continue;
        }
        let mut kept = Vec::with_capacity(block.instructions.len());
        for instruction in std::mem::take(&mut block.instructions) {
            if is_hoistable_instruction(
                &instruction,
                &loop_defined_temps,
                &loop_assigned_locals,
                &hoisted_temps,
            ) {
                remember_hoisted_target(&instruction, &mut hoisted_temps);
                hoisted_instructions.push(instruction);
                changed = true;
            } else {
                kept.push(instruction);
            }
        }
        block.instructions = kept;
    }

    if !hoisted_instructions.is_empty()
        && let Some(preheader) = function
            .blocks
            .iter_mut()
            .find(|block| block.label == natural_loop.preheader)
    {
        preheader.instructions.extend(hoisted_instructions);
    }

    changed
}

fn is_hoistable_instruction(
    instruction: &MirInstruction,
    loop_defined_temps: &HashSet<String>,
    loop_assigned_locals: &HashSet<String>,
    hoisted_temps: &HashSet<String>,
) -> bool {
    match instruction {
        MirInstruction::ConstInt { target, .. } | MirInstruction::ConstBool { target, .. } => {
            matches!(target, MirValue::Temp { .. })
        }
        MirInstruction::Binary {
            target,
            op,
            left,
            right,
        } => {
            matches!(target, MirValue::Temp { .. })
                && matches!(op, MirBinaryOp::Add | MirBinaryOp::Sub | MirBinaryOp::Mul)
                && !is_f64_type(value_type(target))
                && !is_f64_type(value_type(left))
                && !is_f64_type(value_type(right))
                && is_invariant_value(
                    left,
                    loop_defined_temps,
                    loop_assigned_locals,
                    hoisted_temps,
                )
                && is_invariant_value(
                    right,
                    loop_defined_temps,
                    loop_assigned_locals,
                    hoisted_temps,
                )
        }
        MirInstruction::ConstFloat { .. }
        | MirInstruction::Move { .. }
        | MirInstruction::Unary { .. }
        | MirInstruction::Compare { .. }
        | MirInstruction::Cast { .. }
        | MirInstruction::Address { .. }
        | MirInstruction::Load { .. }
        | MirInstruction::Store { .. }
        | MirInstruction::MakeSlice { .. }
        | MirInstruction::SliceData { .. }
        | MirInstruction::SliceLen { .. }
        | MirInstruction::Subslice { .. }
        | MirInstruction::Call { .. } => false,
    }
}

fn is_invariant_value(
    value: &MirValue,
    loop_defined_temps: &HashSet<String>,
    loop_assigned_locals: &HashSet<String>,
    hoisted_temps: &HashSet<String>,
) -> bool {
    match value {
        MirValue::ConstInt { .. }
        | MirValue::ConstBool { .. }
        | MirValue::ConstFloat { .. }
        | MirValue::Param { .. } => true,
        MirValue::Local { name, .. } => !loop_assigned_locals.contains(name),
        MirValue::Temp { name, .. } => {
            !loop_defined_temps.contains(name) || hoisted_temps.contains(name)
        }
    }
}

fn collect_loop_defined_temps(
    function: &MirFunction,
    loop_blocks: &HashSet<String>,
) -> HashSet<String> {
    let mut temps = HashSet::new();
    for block in &function.blocks {
        if !loop_blocks.contains(&block.label) {
            continue;
        }
        for instruction in &block.instructions {
            if let Some(MirValue::Temp { name, .. }) = instruction_target(instruction) {
                temps.insert(name.clone());
            }
        }
    }
    temps
}

fn collect_loop_assigned_locals(
    function: &MirFunction,
    loop_blocks: &HashSet<String>,
) -> HashSet<String> {
    let mut locals = HashSet::new();
    for block in &function.blocks {
        if !loop_blocks.contains(&block.label) {
            continue;
        }
        for instruction in &block.instructions {
            if let Some(MirValue::Local { name, .. }) = instruction_target(instruction) {
                locals.insert(name.clone());
            }
            if let MirInstruction::Store { place, .. } = instruction {
                collect_assigned_place_local(place, &mut locals);
            }
        }
    }
    locals
}

fn collect_assigned_place_local(place: &MirPlace, locals: &mut HashSet<String>) {
    match place {
        MirPlace::Local { name, .. } => {
            locals.insert(name.clone());
        }
        MirPlace::Field { base, .. } | MirPlace::Index { base, .. } => {
            collect_assigned_place_local(base, locals);
        }
        MirPlace::Param { .. } | MirPlace::Deref { .. } | MirPlace::SliceIndex { .. } => {}
    }
}

fn remember_hoisted_target(instruction: &MirInstruction, hoisted_temps: &mut HashSet<String>) {
    if let Some(MirValue::Temp { name, .. }) = instruction_target(instruction) {
        hoisted_temps.insert(name.clone());
    }
}

fn analyze_natural_loops(function: &MirFunction) -> Vec<NaturalLoop> {
    if function.blocks.is_empty() {
        return Vec::new();
    }

    let labels = function
        .blocks
        .iter()
        .map(|block| block.label.clone())
        .collect::<Vec<_>>();
    let label_set = labels.iter().cloned().collect::<HashSet<_>>();
    let successors = build_successors(function);
    let predecessors = build_predecessors(&labels, &successors);
    let dominators = compute_dominators(&labels, &successors, &predecessors);
    let mut loops = Vec::new();

    for block in &function.blocks {
        for target in successors.get(&block.label).into_iter().flatten() {
            if !label_set.contains(target) {
                continue;
            }
            if !dominators
                .get(&block.label)
                .is_some_and(|doms| doms.contains(target))
            {
                continue;
            }

            let loop_blocks = collect_natural_loop_blocks(target, &block.label, &predecessors);
            if let Some(natural_loop) = describe_simple_loop(
                function,
                LoopBackEdge {
                    from: block.label.clone(),
                    to: target.clone(),
                },
                loop_blocks,
                &predecessors,
                &successors,
            ) {
                loops.push(natural_loop);
            }
        }
    }

    let order = block_order(function);
    loops.sort_by_key(|natural_loop| {
        order
            .get(&natural_loop.header)
            .copied()
            .unwrap_or(usize::MAX)
    });
    loops
}

fn describe_simple_loop(
    function: &MirFunction,
    back_edge: LoopBackEdge,
    blocks: HashSet<String>,
    predecessors: &HashMap<String, HashSet<String>>,
    successors: &HashMap<String, Vec<String>>,
) -> Option<NaturalLoop> {
    let header_block = function
        .blocks
        .iter()
        .find(|block| block.label == back_edge.to)?;
    if !matches!(header_block.terminator, MirTerminator::Branch { .. }) {
        return None;
    }

    let outside_header_predecessors = predecessors
        .get(&back_edge.to)
        .into_iter()
        .flatten()
        .filter(|label| !blocks.contains(*label))
        .cloned()
        .collect::<Vec<_>>();
    let [preheader] = outside_header_predecessors.as_slice() else {
        return None;
    };

    let preheader_block = function
        .blocks
        .iter()
        .find(|block| block.label == *preheader)?;
    if !matches!(
        &preheader_block.terminator,
        MirTerminator::Jump { label } if label == preheader_block_successor(&back_edge)
    ) {
        return None;
    }

    let mut exit_blocks = HashSet::new();
    for label in &blocks {
        for successor in successors.get(label).into_iter().flatten() {
            if !blocks.contains(successor) {
                exit_blocks.insert(successor.clone());
            }
        }
    }
    let order = block_order(function);
    let mut exit_blocks = exit_blocks.into_iter().collect::<Vec<_>>();
    exit_blocks.sort_by_key(|label| order.get(label).copied().unwrap_or(usize::MAX));

    Some(NaturalLoop {
        header: back_edge.to.clone(),
        back_edge,
        blocks,
        preheader: preheader.clone(),
        exit_blocks,
    })
}

fn preheader_block_successor(back_edge: &LoopBackEdge) -> &str {
    &back_edge.to
}

fn collect_natural_loop_blocks(
    header: &str,
    source: &str,
    predecessors: &HashMap<String, HashSet<String>>,
) -> HashSet<String> {
    let mut blocks = HashSet::from([header.to_string(), source.to_string()]);
    let mut worklist = vec![source.to_string()];
    while let Some(label) = worklist.pop() {
        for predecessor in predecessors.get(&label).into_iter().flatten() {
            if blocks.insert(predecessor.clone()) {
                worklist.push(predecessor.clone());
            }
        }
    }
    blocks
}

fn compute_dominators(
    labels: &[String],
    successors: &HashMap<String, Vec<String>>,
    predecessors: &HashMap<String, HashSet<String>>,
) -> HashMap<String, HashSet<String>> {
    let Some(entry) = labels.first() else {
        return HashMap::new();
    };
    let all_labels = labels.iter().cloned().collect::<HashSet<_>>();
    let mut dominators = HashMap::new();
    for label in labels {
        dominators.insert(
            label.clone(),
            if label == entry {
                HashSet::from([entry.clone()])
            } else {
                all_labels.clone()
            },
        );
    }

    let mut changed = true;
    while changed {
        changed = false;
        for label in labels.iter().skip(1) {
            let preds = predecessors
                .get(label)
                .into_iter()
                .flatten()
                .filter(|pred| dominators.contains_key(*pred))
                .collect::<Vec<_>>();
            let mut next = all_labels.clone();
            for pred in preds {
                if let Some(pred_dominators) = dominators.get(pred) {
                    next.retain(|entry| pred_dominators.contains(entry));
                }
            }
            next.insert(label.clone());
            if dominators.get(label) != Some(&next) {
                dominators.insert(label.clone(), next);
                changed = true;
            }
        }
    }

    for (label, successor_list) in successors {
        dominators
            .entry(label.clone())
            .or_insert_with(|| successor_list.iter().cloned().collect());
    }
    dominators
}

fn build_successors(function: &MirFunction) -> HashMap<String, Vec<String>> {
    function
        .blocks
        .iter()
        .map(|block| (block.label.clone(), terminator_targets(&block.terminator)))
        .collect()
}

fn build_predecessors(
    labels: &[String],
    successors: &HashMap<String, Vec<String>>,
) -> HashMap<String, HashSet<String>> {
    let mut predecessors = labels
        .iter()
        .map(|label| (label.clone(), HashSet::new()))
        .collect::<HashMap<_, _>>();
    for (label, successor_list) in successors {
        for successor in successor_list {
            if let Some(preds) = predecessors.get_mut(successor) {
                preds.insert(label.clone());
            }
        }
    }
    predecessors
}

fn block_order(function: &MirFunction) -> HashMap<String, usize> {
    function
        .blocks
        .iter()
        .enumerate()
        .map(|(index, block)| (block.label.clone(), index))
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CseEntry {
    value: MirValue,
    dependencies: HashSet<String>,
}

fn run_local_cse(module: &mut MirModule, context: &MirPassContext) -> MirPassResult {
    if context.bounds_mode == MirPassBoundsMode::Checked {
        return MirPassResult {
            changed: false,
            diagnostics: Vec::new(),
        };
    }
    let mut changed = false;

    for function in &mut module.functions {
        for block in &mut function.blocks {
            let mut expressions: HashMap<String, CseEntry> = HashMap::new();

            for instruction in &mut block.instructions {
                if matches!(
                    instruction,
                    MirInstruction::Store { .. } | MirInstruction::Call { .. }
                ) {
                    expressions.clear();
                }

                let key = cse_key(instruction);
                let target = instruction_target(instruction).cloned();
                if let (Some(key), Some(target @ MirValue::Temp { .. })) = (key, target.clone()) {
                    if let Some(existing) = expressions.get(&key) {
                        *instruction = MirInstruction::Move {
                            target,
                            value: existing.value.clone(),
                        };
                        changed = true;
                    } else {
                        expressions.insert(
                            key,
                            CseEntry {
                                value: target,
                                dependencies: collect_instruction_dependencies(instruction),
                            },
                        );
                    }
                }

                if let Some(target) = target {
                    match target {
                        MirValue::Local { name, .. } => {
                            invalidate_cse_dependency(&mut expressions, &format!("local:{name}"));
                        }
                        MirValue::Param { name, .. } => {
                            invalidate_cse_dependency(&mut expressions, &format!("param:{name}"));
                        }
                        MirValue::Temp { .. }
                        | MirValue::ConstInt { .. }
                        | MirValue::ConstFloat { .. }
                        | MirValue::ConstBool { .. } => {}
                    }
                }
            }
        }
    }

    MirPassResult {
        changed,
        diagnostics: Vec::new(),
    }
}

fn cse_key(instruction: &MirInstruction) -> Option<String> {
    match instruction {
        MirInstruction::Binary {
            target,
            op,
            left,
            right,
        } => {
            if is_f64_type(value_type(target)) {
                return float_binary_cse_key(*op, target, left, right);
            }
            let (left_key, right_key) = ordered_value_keys(*op, left, right);
            Some(format!(
                "binary:{}:{}:{left_key}:{right_key}",
                binary_op_key(*op),
                type_key(value_type(target))
            ))
        }
        MirInstruction::Compare {
            target: _,
            op,
            left,
            right,
        } => {
            if is_f64_type(value_type(left)) || is_f64_type(value_type(right)) {
                return None;
            }
            let (left_key, right_key) = ordered_compare_value_keys(*op, left, right);
            Some(format!(
                "compare:{}:{}:{left_key}:{right_key}",
                compare_op_key(*op),
                type_key(value_type(left))
            ))
        }
        MirInstruction::Unary {
            target,
            op,
            operand,
        } => {
            if is_f64_type(value_type(target)) || is_f64_type(value_type(operand)) {
                return float_unary_cse_key(*op, target, operand);
            }
            Some(format!(
                "unary:{}:{}:{}",
                unary_op_key(*op),
                type_key(value_type(target)),
                value_key(operand)
            ))
        }
        MirInstruction::Cast { target, op, value } => Some(format!(
            "cast:{}:{}:{}:{}",
            cast_op_key(*op),
            type_key(value_type(value)),
            type_key(value_type(target)),
            value_key(value)
        )),
        MirInstruction::ConstInt { .. }
        | MirInstruction::ConstFloat { .. }
        | MirInstruction::ConstBool { .. }
        | MirInstruction::Move { .. }
        | MirInstruction::Address { .. }
        | MirInstruction::Load { .. }
        | MirInstruction::Store { .. }
        | MirInstruction::MakeSlice { .. }
        | MirInstruction::SliceData { .. }
        | MirInstruction::SliceLen { .. }
        | MirInstruction::Subslice { .. }
        | MirInstruction::Call { .. } => None,
    }
}

fn float_binary_cse_key(
    op: MirBinaryOp,
    target: &MirValue,
    left: &MirValue,
    right: &MirValue,
) -> Option<String> {
    if !is_f64_type(value_type(target))
        || !is_f64_type(value_type(left))
        || !is_f64_type(value_type(right))
    {
        return None;
    }
    if !matches!(op, MirBinaryOp::Add | MirBinaryOp::Sub | MirBinaryOp::Mul) {
        return None;
    }
    Some(format!(
        "float-binary:{}:{}:{}:{}",
        binary_op_key(op),
        type_key(value_type(target)),
        value_key(left),
        value_key(right)
    ))
}

fn float_unary_cse_key(op: MirUnaryOp, target: &MirValue, operand: &MirValue) -> Option<String> {
    if !is_f64_type(value_type(target))
        || !is_f64_type(value_type(operand))
        || op != MirUnaryOp::Neg
    {
        return None;
    }
    Some(format!(
        "float-unary:{}:{}:{}",
        unary_op_key(op),
        type_key(value_type(target)),
        value_key(operand)
    ))
}

fn ordered_value_keys(op: MirBinaryOp, left: &MirValue, right: &MirValue) -> (String, String) {
    let left_key = value_key(left);
    let right_key = value_key(right);
    if matches!(op, MirBinaryOp::Add | MirBinaryOp::Mul) && right_key < left_key {
        (right_key, left_key)
    } else {
        (left_key, right_key)
    }
}

fn ordered_compare_value_keys(
    op: MirCompareOp,
    left: &MirValue,
    right: &MirValue,
) -> (String, String) {
    let left_key = value_key(left);
    let right_key = value_key(right);
    if matches!(op, MirCompareOp::Eq | MirCompareOp::Ne) && right_key < left_key {
        (right_key, left_key)
    } else {
        (left_key, right_key)
    }
}

fn collect_instruction_dependencies(instruction: &MirInstruction) -> HashSet<String> {
    let mut dependencies = HashSet::new();
    match instruction {
        MirInstruction::Binary { left, right, .. }
        | MirInstruction::Compare { left, right, .. } => {
            collect_value_dependency(left, &mut dependencies);
            collect_value_dependency(right, &mut dependencies);
        }
        MirInstruction::Unary { operand, .. } => {
            collect_value_dependency(operand, &mut dependencies)
        }
        MirInstruction::Cast { value, .. } => collect_value_dependency(value, &mut dependencies),
        MirInstruction::MakeSlice { data, len, .. } => {
            collect_value_dependency(data, &mut dependencies);
            collect_value_dependency(len, &mut dependencies);
        }
        MirInstruction::SliceData { slice, .. } | MirInstruction::SliceLen { slice, .. } => {
            collect_value_dependency(slice, &mut dependencies);
        }
        MirInstruction::Subslice {
            slice, start, end, ..
        } => {
            collect_value_dependency(slice, &mut dependencies);
            collect_value_dependency(start, &mut dependencies);
            collect_value_dependency(end, &mut dependencies);
        }
        MirInstruction::ConstInt { .. }
        | MirInstruction::ConstFloat { .. }
        | MirInstruction::ConstBool { .. }
        | MirInstruction::Move { .. }
        | MirInstruction::Address { .. }
        | MirInstruction::Load { .. }
        | MirInstruction::Store { .. }
        | MirInstruction::Call { .. } => {}
    }
    dependencies
}

fn collect_value_dependency(value: &MirValue, dependencies: &mut HashSet<String>) {
    match value {
        MirValue::Local { name, .. } => {
            dependencies.insert(format!("local:{name}"));
        }
        MirValue::Param { name, .. } => {
            dependencies.insert(format!("param:{name}"));
        }
        MirValue::Temp { .. }
        | MirValue::ConstInt { .. }
        | MirValue::ConstFloat { .. }
        | MirValue::ConstBool { .. } => {}
    }
}

fn invalidate_cse_dependency(expressions: &mut HashMap<String, CseEntry>, dependency: &str) {
    expressions.retain(|_, entry| !entry.dependencies.contains(dependency));
}

fn value_key(value: &MirValue) -> String {
    match value {
        MirValue::Param { name, type_node } => format!("param:{name}:{}", type_key(type_node)),
        MirValue::Local { name, type_node } => format!("local:{name}:{}", type_key(type_node)),
        MirValue::Temp { name, type_node } => format!("temp:{name}:{}", type_key(type_node)),
        MirValue::ConstInt { text, type_node } => {
            format!("const_int:{text}:{}", type_key(type_node))
        }
        MirValue::ConstFloat { text, type_node } => {
            format!("const_float:{text}:{}", type_key(type_node))
        }
        MirValue::ConstBool { value, .. } => format!("const_bool:{value}"),
    }
}

fn type_key(type_node: &MirType) -> String {
    match type_node {
        MirType::Primitive(name) => primitive_type_key(*name).to_string(),
        MirType::Pointer(element_type) => format!("ptr<{}>", type_key(element_type)),
        MirType::Slice(element_type) => format!("slice<{}>", type_key(element_type)),
        MirType::Struct(name) => format!("struct:{name}"),
        MirType::Void => "void".to_string(),
    }
}

fn primitive_type_key(name: MirPrimitiveTypeName) -> &'static str {
    match name {
        MirPrimitiveTypeName::I32 => "i32",
        MirPrimitiveTypeName::I64 => "i64",
        MirPrimitiveTypeName::U32 => "u32",
        MirPrimitiveTypeName::U64 => "u64",
        MirPrimitiveTypeName::F64 => "f64",
        MirPrimitiveTypeName::Bool => "bool",
    }
}

fn binary_op_key(op: MirBinaryOp) -> &'static str {
    match op {
        MirBinaryOp::Add => "+",
        MirBinaryOp::Sub => "-",
        MirBinaryOp::Mul => "*",
        MirBinaryOp::Div => "/",
        MirBinaryOp::Mod => "%",
    }
}

fn compare_op_key(op: MirCompareOp) -> &'static str {
    match op {
        MirCompareOp::Eq => "==",
        MirCompareOp::Ne => "!=",
        MirCompareOp::Lt => "<",
        MirCompareOp::Le => "<=",
        MirCompareOp::Gt => ">",
        MirCompareOp::Ge => ">=",
    }
}

fn unary_op_key(op: MirUnaryOp) -> &'static str {
    match op {
        MirUnaryOp::Neg => "neg",
        MirUnaryOp::Not => "not",
    }
}

fn cast_op_key(op: crate::MirCastOp) -> &'static str {
    match op {
        crate::MirCastOp::I32ToF64 => "i32_to_f64",
        crate::MirCastOp::U32ToF64 => "u32_to_f64",
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AddressEntry {
    pointer: MirValue,
    dependencies: HashSet<String>,
}

fn run_address_cse(module: &mut MirModule, context: &MirPassContext) -> MirPassResult {
    if context.bounds_mode == MirPassBoundsMode::Checked
        || !matches!(
            context.target_backend,
            MirPassTargetBackend::C | MirPassTargetBackend::Wasm
        )
    {
        return MirPassResult {
            changed: false,
            diagnostics: Vec::new(),
        };
    }

    let mut changed = false;
    for function in &mut module.functions {
        let mut allocator = AddressTempAllocator::new(function);

        for block in &mut function.blocks {
            let mut addresses: HashMap<String, AddressEntry> = HashMap::new();
            let mut next_instructions = Vec::with_capacity(block.instructions.len());

            for instruction in std::mem::take(&mut block.instructions) {
                if matches!(instruction, MirInstruction::Call { .. }) {
                    addresses.clear();
                    next_instructions.push(instruction);
                    continue;
                }

                let original = instruction.clone();
                let mut inserted = Vec::new();
                let rewritten = rewrite_address_instruction(
                    instruction,
                    &mut addresses,
                    &mut allocator,
                    &mut inserted,
                );
                if !inserted.is_empty() || rewritten != original {
                    changed = true;
                }
                next_instructions.extend(inserted);

                let is_store = matches!(rewritten, MirInstruction::Store { .. });
                let target = instruction_target(&rewritten).cloned();
                next_instructions.push(rewritten);

                if is_store {
                    addresses.clear();
                    continue;
                }

                if let Some(target) = target {
                    match target {
                        MirValue::Local { name, .. } => {
                            invalidate_address_dependency(&mut addresses, &format!("local:{name}"));
                        }
                        MirValue::Param { name, .. } => {
                            invalidate_address_dependency(&mut addresses, &format!("param:{name}"));
                        }
                        MirValue::Temp { .. }
                        | MirValue::ConstInt { .. }
                        | MirValue::ConstFloat { .. }
                        | MirValue::ConstBool { .. } => {}
                    }
                }
            }

            block.instructions = next_instructions;
        }
    }

    MirPassResult {
        changed,
        diagnostics: Vec::new(),
    }
}

fn rewrite_address_instruction(
    instruction: MirInstruction,
    addresses: &mut HashMap<String, AddressEntry>,
    allocator: &mut AddressTempAllocator,
    inserted: &mut Vec<MirInstruction>,
) -> MirInstruction {
    match instruction {
        MirInstruction::Load { target, place } => MirInstruction::Load {
            target,
            place: rewrite_address_place(place, addresses, allocator, inserted),
        },
        MirInstruction::Store { place, value } => MirInstruction::Store {
            place: rewrite_address_place(place, addresses, allocator, inserted),
            value,
        },
        MirInstruction::Address { target, place } => MirInstruction::Address {
            target,
            place: rewrite_address_place(place, addresses, allocator, inserted),
        },
        other => other,
    }
}

fn rewrite_address_place(
    place: MirPlace,
    addresses: &mut HashMap<String, AddressEntry>,
    allocator: &mut AddressTempAllocator,
    inserted: &mut Vec<MirInstruction>,
) -> MirPlace {
    match place {
        MirPlace::Field {
            base,
            field_name,
            type_node,
        } => {
            if is_indexed_struct_place(&base) {
                let base_type = place_type(&base).clone();
                let pointer = pointer_for_indexed_place(*base, addresses, allocator, inserted);
                return MirPlace::Field {
                    base: Box::new(MirPlace::Deref {
                        pointer,
                        type_node: base_type,
                    }),
                    field_name,
                    type_node,
                };
            }
            MirPlace::Field {
                base: Box::new(rewrite_address_place(*base, addresses, allocator, inserted)),
                field_name,
                type_node,
            }
        }
        MirPlace::Index {
            base,
            index,
            type_node,
        } => {
            let place = MirPlace::Index {
                base,
                index,
                type_node,
            };
            if should_materialize_indexed_place(&place) {
                let deref_type = place_type(&place).clone();
                let pointer = pointer_for_indexed_place(place, addresses, allocator, inserted);
                MirPlace::Deref {
                    pointer,
                    type_node: deref_type,
                }
            } else if let MirPlace::Index {
                base,
                index,
                type_node,
            } = place
            {
                MirPlace::Index {
                    base: Box::new(rewrite_address_place(*base, addresses, allocator, inserted)),
                    index,
                    type_node,
                }
            } else {
                unreachable!()
            }
        }
        MirPlace::SliceIndex { .. } => place,
        MirPlace::Deref { .. } | MirPlace::Param { .. } | MirPlace::Local { .. } => place,
    }
}

fn pointer_for_indexed_place(
    place: MirPlace,
    addresses: &mut HashMap<String, AddressEntry>,
    allocator: &mut AddressTempAllocator,
    inserted: &mut Vec<MirInstruction>,
) -> MirValue {
    let key = indexed_place_key(&place);
    if let Some(entry) = addresses.get(&key) {
        return entry.pointer.clone();
    }

    let pointer = allocator.next(place_type(&place).clone());
    inserted.push(MirInstruction::Address {
        target: pointer.clone(),
        place: place.clone(),
    });
    addresses.insert(
        key,
        AddressEntry {
            pointer: pointer.clone(),
            dependencies: collect_place_dependencies(&place),
        },
    );
    pointer
}

fn is_indexed_struct_place(place: &MirPlace) -> bool {
    matches!(
        place,
        MirPlace::Index {
            type_node: MirType::Struct(_),
            ..
        }
    )
}

fn should_materialize_indexed_place(place: &MirPlace) -> bool {
    matches!(place, MirPlace::Index { type_node, .. } if !matches!(type_node, MirType::Struct(_)))
}

fn indexed_place_key(place: &MirPlace) -> String {
    format!("indexed:{}", place_key(place))
}

#[derive(Debug, Clone)]
struct AddressTempAllocator {
    used: HashSet<String>,
    index: usize,
}

impl AddressTempAllocator {
    fn new(function: &MirFunction) -> Self {
        let mut used = HashSet::new();
        for block in &function.blocks {
            for instruction in &block.instructions {
                if let Some(MirValue::Temp { name, .. }) = instruction_target(instruction) {
                    used.insert(name.clone());
                }
            }
        }
        Self { used, index: 0 }
    }

    fn next(&mut self, element_type: MirType) -> MirValue {
        while self.used.contains(&format!("addr{}", self.index)) {
            self.index += 1;
        }
        let name = format!("addr{}", self.index);
        self.index += 1;
        self.used.insert(name.clone());
        MirValue::Temp {
            name,
            type_node: MirType::Pointer(Box::new(element_type)),
        }
    }
}

fn invalidate_address_dependency(
    expressions: &mut HashMap<String, AddressEntry>,
    dependency: &str,
) {
    expressions.retain(|_, entry| !entry.dependencies.contains(dependency));
}

fn collect_place_dependencies(place: &MirPlace) -> HashSet<String> {
    let mut dependencies = HashSet::new();
    collect_place_dependency(place, &mut dependencies);
    dependencies
}

fn collect_place_dependency(place: &MirPlace, dependencies: &mut HashSet<String>) {
    match place {
        MirPlace::Param { name, .. } => {
            dependencies.insert(format!("param:{name}"));
        }
        MirPlace::Local { name, .. } => {
            dependencies.insert(format!("local:{name}"));
        }
        MirPlace::Deref { pointer, .. } => collect_value_dependency(pointer, dependencies),
        MirPlace::Index { base, index, .. } => {
            collect_place_dependency(base, dependencies);
            collect_value_dependency(index, dependencies);
        }
        MirPlace::SliceIndex { slice, index, .. } => {
            collect_value_dependency(slice, dependencies);
            collect_value_dependency(index, dependencies);
        }
        MirPlace::Field { base, .. } => collect_place_dependency(base, dependencies),
    }
}

fn place_key(place: &MirPlace) -> String {
    match place {
        MirPlace::Param { name, type_node } => format!("param:{name}:{}", type_key(type_node)),
        MirPlace::Local { name, type_node } => format!("local:{name}:{}", type_key(type_node)),
        MirPlace::Deref { pointer, type_node } => {
            format!("deref:{}:{}", value_key(pointer), type_key(type_node))
        }
        MirPlace::Index {
            base,
            index,
            type_node,
        } => format!(
            "index:{}:{}:{}",
            place_key(base),
            value_key(index),
            type_key(type_node)
        ),
        MirPlace::SliceIndex {
            slice,
            index,
            type_node,
        } => format!(
            "slice_index:{}:{}:{}",
            value_key(slice),
            value_key(index),
            type_key(type_node)
        ),
        MirPlace::Field {
            base,
            field_name,
            type_node,
        } => format!(
            "field:{}:{field_name}:{}",
            place_key(base),
            type_key(type_node)
        ),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum KnownConstant {
    Int { value: i128, type_node: MirType },
    Bool { value: bool, type_node: MirType },
}

fn run_constant_folding(module: &mut MirModule, context: &MirPassContext) -> MirPassResult {
    if context.overflow_mode == MirPassOverflowMode::Checked {
        return MirPassResult {
            changed: false,
            diagnostics: Vec::new(),
        };
    }

    let mut changed = false;
    for function in &mut module.functions {
        for block in &mut function.blocks {
            let mut constants = HashMap::new();
            for instruction in &mut block.instructions {
                if let Some(folded) = fold_instruction(instruction, &constants) {
                    *instruction = folded;
                    remember_instruction_constant(instruction, &mut constants);
                    changed = true;
                } else {
                    forget_instruction_target(instruction, &mut constants);
                    remember_instruction_constant(instruction, &mut constants);
                }
            }
        }
    }

    MirPassResult {
        changed,
        diagnostics: Vec::new(),
    }
}

fn fold_instruction(
    instruction: &MirInstruction,
    constants: &HashMap<String, KnownConstant>,
) -> Option<MirInstruction> {
    match instruction {
        MirInstruction::Binary {
            target,
            op,
            left,
            right,
        } => {
            let left = get_known_constant(left, constants)?;
            let right = get_known_constant(right, constants)?;
            let (KnownConstant::Int { value: left, .. }, KnownConstant::Int { value: right, .. }) =
                (left, right)
            else {
                return None;
            };
            fold_binary(*op, left, right, value_type(target)).map(|value| {
                MirInstruction::ConstInt {
                    target: target.clone(),
                    value: value.to_string(),
                }
            })
        }
        MirInstruction::Compare {
            target,
            op,
            left,
            right,
        } => {
            let left = get_known_constant(left, constants)?;
            let right = get_known_constant(right, constants)?;
            let value = match (left, right) {
                (
                    KnownConstant::Int { value: left, .. },
                    KnownConstant::Int { value: right, .. },
                ) => fold_int_compare(*op, left, right),
                (
                    KnownConstant::Bool { value: left, .. },
                    KnownConstant::Bool { value: right, .. },
                ) => fold_bool_compare(*op, left, right),
                _ => None,
            }?;
            Some(MirInstruction::ConstBool {
                target: target.clone(),
                value,
            })
        }
        MirInstruction::Unary {
            target,
            op,
            operand,
        } => fold_unary(*op, get_known_constant(operand, constants)?, target),
        MirInstruction::ConstInt { .. }
        | MirInstruction::ConstFloat { .. }
        | MirInstruction::ConstBool { .. }
        | MirInstruction::Move { .. }
        | MirInstruction::Cast { .. }
        | MirInstruction::Address { .. }
        | MirInstruction::Load { .. }
        | MirInstruction::Store { .. }
        | MirInstruction::MakeSlice { .. }
        | MirInstruction::SliceData { .. }
        | MirInstruction::SliceLen { .. }
        | MirInstruction::Subslice { .. }
        | MirInstruction::Call { .. } => None,
    }
}

fn remember_instruction_constant(
    instruction: &MirInstruction,
    constants: &mut HashMap<String, KnownConstant>,
) {
    match instruction {
        MirInstruction::ConstInt { target, value } => {
            if let (Some(name), Ok(value)) = (temp_name(target), value.parse::<i128>()) {
                constants.insert(
                    name.to_string(),
                    KnownConstant::Int {
                        value,
                        type_node: value_type(target).clone(),
                    },
                );
            }
        }
        MirInstruction::ConstBool { target, value } => {
            if let Some(name) = temp_name(target) {
                constants.insert(
                    name.to_string(),
                    KnownConstant::Bool {
                        value: *value,
                        type_node: value_type(target).clone(),
                    },
                );
            }
        }
        MirInstruction::ConstFloat { .. }
        | MirInstruction::Move { .. }
        | MirInstruction::Binary { .. }
        | MirInstruction::Unary { .. }
        | MirInstruction::Compare { .. }
        | MirInstruction::Cast { .. }
        | MirInstruction::Address { .. }
        | MirInstruction::Load { .. }
        | MirInstruction::Store { .. }
        | MirInstruction::MakeSlice { .. }
        | MirInstruction::SliceData { .. }
        | MirInstruction::SliceLen { .. }
        | MirInstruction::Subslice { .. }
        | MirInstruction::Call { .. } => {}
    }
}

fn forget_instruction_target(
    instruction: &MirInstruction,
    constants: &mut HashMap<String, KnownConstant>,
) {
    if let Some(target) = instruction_target(instruction)
        && let Some(name) = temp_name(target)
    {
        constants.remove(name);
    }
}

fn get_known_constant(
    value: &MirValue,
    constants: &HashMap<String, KnownConstant>,
) -> Option<KnownConstant> {
    match value {
        MirValue::ConstInt { text, type_node } => {
            text.parse::<i128>().ok().map(|value| KnownConstant::Int {
                value,
                type_node: type_node.clone(),
            })
        }
        MirValue::ConstBool { value, type_node } => Some(KnownConstant::Bool {
            value: *value,
            type_node: type_node.clone(),
        }),
        MirValue::Temp { name, .. } => constants.get(name).cloned(),
        MirValue::ConstFloat { .. } | MirValue::Param { .. } | MirValue::Local { .. } => None,
    }
}

fn fold_binary(op: MirBinaryOp, left: i128, right: i128, type_node: &MirType) -> Option<i128> {
    if !is_integer_type(type_node) {
        return None;
    }
    if matches!(op, MirBinaryOp::Div | MirBinaryOp::Mod) && right == 0 {
        return None;
    }
    if matches!(op, MirBinaryOp::Div | MirBinaryOp::Mod)
        && is_signed_integer_type(type_node)
        && left == integer_min(type_node)?
        && right == -1
    {
        return None;
    }

    let result = match op {
        MirBinaryOp::Add => left.checked_add(right)?,
        MirBinaryOp::Sub => left.checked_sub(right)?,
        MirBinaryOp::Mul => left.checked_mul(right)?,
        MirBinaryOp::Div => left.checked_div(right)?,
        MirBinaryOp::Mod => left.checked_rem(right)?,
    };

    fits_integer_type(result, type_node).then_some(result)
}

fn fold_unary(op: MirUnaryOp, operand: KnownConstant, target: &MirValue) -> Option<MirInstruction> {
    match op {
        MirUnaryOp::Not => match operand {
            KnownConstant::Bool { value, .. } => Some(MirInstruction::ConstBool {
                target: target.clone(),
                value: !value,
            }),
            KnownConstant::Int { .. } => None,
        },
        MirUnaryOp::Neg => {
            let KnownConstant::Int { value, .. } = operand else {
                return None;
            };
            if !is_integer_type(value_type(target)) {
                return None;
            }
            let value = value.checked_neg()?;
            fits_integer_type(value, value_type(target)).then(|| MirInstruction::ConstInt {
                target: target.clone(),
                value: value.to_string(),
            })
        }
    }
}

fn fold_int_compare(op: MirCompareOp, left: i128, right: i128) -> Option<bool> {
    Some(match op {
        MirCompareOp::Eq => left == right,
        MirCompareOp::Ne => left != right,
        MirCompareOp::Lt => left < right,
        MirCompareOp::Le => left <= right,
        MirCompareOp::Gt => left > right,
        MirCompareOp::Ge => left >= right,
    })
}

fn fold_bool_compare(op: MirCompareOp, left: bool, right: bool) -> Option<bool> {
    match op {
        MirCompareOp::Eq => Some(left == right),
        MirCompareOp::Ne => Some(left != right),
        MirCompareOp::Lt | MirCompareOp::Le | MirCompareOp::Gt | MirCompareOp::Ge => None,
    }
}

fn run_copy_propagation(module: &mut MirModule, _context: &MirPassContext) -> MirPassResult {
    let mut changed = false;
    for function in &mut module.functions {
        for block in &mut function.blocks {
            let mut copies = HashMap::new();
            for instruction in &mut block.instructions {
                changed |= rewrite_instruction_copies(instruction, &copies);

                if matches!(
                    instruction,
                    MirInstruction::Call { .. } | MirInstruction::Store { .. }
                ) {
                    copies.clear();
                    continue;
                }

                if let Some(target) = instruction_target(instruction)
                    && let Some(name) = temp_name(target)
                {
                    copies.remove(name);
                }

                if let MirInstruction::Move { target, value } = instruction
                    && let Some(name) = temp_name(target)
                {
                    copies.insert(name.to_string(), value.clone());
                }
            }
            changed |= rewrite_terminator_copies(&mut block.terminator, &copies);
        }
    }

    MirPassResult {
        changed,
        diagnostics: Vec::new(),
    }
}

fn rewrite_instruction_copies(
    instruction: &mut MirInstruction,
    copies: &HashMap<String, MirValue>,
) -> bool {
    let mut changed = false;
    match instruction {
        MirInstruction::ConstInt { .. }
        | MirInstruction::ConstFloat { .. }
        | MirInstruction::ConstBool { .. } => false,
        MirInstruction::Move { value, .. } | MirInstruction::Cast { value, .. } => {
            changed |= rewrite_value_copy(value, copies);
            changed
        }
        MirInstruction::Binary { left, right, .. }
        | MirInstruction::Compare { left, right, .. } => {
            changed |= rewrite_value_copy(left, copies);
            changed |= rewrite_value_copy(right, copies);
            changed
        }
        MirInstruction::Unary { operand, .. } => {
            changed |= rewrite_value_copy(operand, copies);
            changed
        }
        MirInstruction::MakeSlice { data, len, .. } => {
            changed |= rewrite_value_copy(data, copies);
            changed |= rewrite_value_copy(len, copies);
            changed
        }
        MirInstruction::SliceData { slice, .. } | MirInstruction::SliceLen { slice, .. } => {
            rewrite_value_copy(slice, copies)
        }
        MirInstruction::Subslice {
            slice, start, end, ..
        } => {
            changed |= rewrite_value_copy(slice, copies);
            changed |= rewrite_value_copy(start, copies);
            changed |= rewrite_value_copy(end, copies);
            changed
        }
        MirInstruction::Address { place, .. } | MirInstruction::Load { place, .. } => {
            rewrite_place_copies(place, copies)
        }
        MirInstruction::Store { place, value } => {
            changed |= rewrite_place_copies(place, copies);
            changed |= rewrite_value_copy(value, copies);
            changed
        }
        MirInstruction::Call { args, .. } => {
            for arg in args {
                changed |= rewrite_value_copy(arg, copies);
            }
            changed
        }
    }
}

fn rewrite_terminator_copies(
    terminator: &mut MirTerminator,
    copies: &HashMap<String, MirValue>,
) -> bool {
    match terminator {
        MirTerminator::Return { value } => value
            .as_mut()
            .is_some_and(|value| rewrite_value_copy(value, copies)),
        MirTerminator::Branch { condition, .. } => rewrite_value_copy(condition, copies),
        MirTerminator::Jump { .. } => false,
    }
}

fn rewrite_place_copies(place: &mut MirPlace, copies: &HashMap<String, MirValue>) -> bool {
    match place {
        MirPlace::Param { .. } | MirPlace::Local { .. } => false,
        MirPlace::Deref { pointer, .. } => rewrite_value_copy(pointer, copies),
        MirPlace::Field { base, .. } => rewrite_place_copies(base, copies),
        MirPlace::Index { base, index, .. } => {
            rewrite_place_copies(base, copies) | rewrite_value_copy(index, copies)
        }
        MirPlace::SliceIndex { slice, index, .. } => {
            rewrite_value_copy(slice, copies) | rewrite_value_copy(index, copies)
        }
    }
}

fn rewrite_value_copy(value: &mut MirValue, copies: &HashMap<String, MirValue>) -> bool {
    let resolved = resolve_copy(value, copies);
    if *value == resolved {
        return false;
    }
    *value = resolved;
    true
}

fn resolve_copy(value: &MirValue, copies: &HashMap<String, MirValue>) -> MirValue {
    let mut current = value.clone();
    let mut seen = HashSet::new();
    while let MirValue::Temp { name, .. } = &current {
        if !seen.insert(name.clone()) {
            return current;
        }
        let Some(next) = copies.get(name) else {
            return current;
        };
        current = next.clone();
    }
    current
}

fn run_dead_code_elimination(module: &mut MirModule, context: &MirPassContext) -> MirPassResult {
    let mut changed = false;
    for function in &mut module.functions {
        changed |= eliminate_dead_code_in_function(function, context);
    }
    MirPassResult {
        changed,
        diagnostics: Vec::new(),
    }
}

fn eliminate_dead_code_in_function(function: &mut MirFunction, context: &MirPassContext) -> bool {
    let mut changed = false;
    let mut removed = true;
    while removed {
        removed = false;
        let used_temps = collect_used_temps(function);
        for block in &mut function.blocks {
            let before = block.instructions.len();
            block.instructions.retain(|instruction| {
                !is_removable_unused_instruction(instruction, &used_temps, context)
            });
            if block.instructions.len() != before {
                removed = true;
                changed = true;
            }
        }
    }
    changed
}

fn is_removable_unused_instruction(
    instruction: &MirInstruction,
    used_temps: &HashSet<String>,
    context: &MirPassContext,
) -> bool {
    if !is_pure_removable_instruction(instruction, context) {
        return false;
    }
    instruction_target(instruction)
        .and_then(temp_name)
        .is_some_and(|name| !used_temps.contains(name))
}

fn is_pure_removable_instruction(instruction: &MirInstruction, context: &MirPassContext) -> bool {
    if context.overflow_mode == MirPassOverflowMode::Checked
        && matches!(
            instruction,
            MirInstruction::Binary { .. } | MirInstruction::Unary { .. }
        )
    {
        return false;
    }
    if context.bounds_mode == MirPassBoundsMode::Checked
        && matches!(
            instruction,
            MirInstruction::Address { place, .. } if place_contains_slice_index(place)
        )
    {
        return false;
    }
    matches!(
        instruction,
        MirInstruction::ConstInt { .. }
            | MirInstruction::ConstFloat { .. }
            | MirInstruction::ConstBool { .. }
            | MirInstruction::Move { .. }
            | MirInstruction::Binary { .. }
            | MirInstruction::Unary { .. }
            | MirInstruction::Compare { .. }
            | MirInstruction::Cast { .. }
            | MirInstruction::Address { .. }
    )
}

fn place_contains_slice_index(place: &MirPlace) -> bool {
    match place {
        MirPlace::SliceIndex { .. } => true,
        MirPlace::Field { base, .. } | MirPlace::Index { base, .. } => {
            place_contains_slice_index(base)
        }
        MirPlace::Param { .. } | MirPlace::Local { .. } | MirPlace::Deref { .. } => false,
    }
}

fn collect_used_temps(function: &MirFunction) -> HashSet<String> {
    let mut used = HashSet::new();
    for block in &function.blocks {
        for instruction in &block.instructions {
            collect_instruction_uses(instruction, &mut used);
        }
        collect_terminator_uses(&block.terminator, &mut used);
    }
    used
}

fn collect_instruction_uses(instruction: &MirInstruction, used: &mut HashSet<String>) {
    match instruction {
        MirInstruction::ConstInt { .. }
        | MirInstruction::ConstFloat { .. }
        | MirInstruction::ConstBool { .. } => {}
        MirInstruction::Move { value, .. } | MirInstruction::Cast { value, .. } => {
            collect_value_use(value, used);
        }
        MirInstruction::Binary { left, right, .. }
        | MirInstruction::Compare { left, right, .. } => {
            collect_value_use(left, used);
            collect_value_use(right, used);
        }
        MirInstruction::Unary { operand, .. } => collect_value_use(operand, used),
        MirInstruction::MakeSlice { data, len, .. } => {
            collect_value_use(data, used);
            collect_value_use(len, used);
        }
        MirInstruction::SliceData { slice, .. } | MirInstruction::SliceLen { slice, .. } => {
            collect_value_use(slice, used);
        }
        MirInstruction::Subslice {
            slice, start, end, ..
        } => {
            collect_value_use(slice, used);
            collect_value_use(start, used);
            collect_value_use(end, used);
        }
        MirInstruction::Address { place, .. } | MirInstruction::Load { place, .. } => {
            collect_place_uses(place, used);
        }
        MirInstruction::Store { place, value } => {
            collect_place_uses(place, used);
            collect_value_use(value, used);
        }
        MirInstruction::Call { args, .. } => {
            for arg in args {
                collect_value_use(arg, used);
            }
        }
    }
}

fn collect_terminator_uses(terminator: &MirTerminator, used: &mut HashSet<String>) {
    match terminator {
        MirTerminator::Return { value } => {
            if let Some(value) = value {
                collect_value_use(value, used);
            }
        }
        MirTerminator::Branch { condition, .. } => collect_value_use(condition, used),
        MirTerminator::Jump { .. } => {}
    }
}

fn collect_place_uses(place: &MirPlace, used: &mut HashSet<String>) {
    match place {
        MirPlace::Param { .. } | MirPlace::Local { .. } => {}
        MirPlace::Deref { pointer, .. } => collect_value_use(pointer, used),
        MirPlace::Field { base, .. } => collect_place_uses(base, used),
        MirPlace::Index { base, index, .. } => {
            collect_place_uses(base, used);
            collect_value_use(index, used);
        }
        MirPlace::SliceIndex { slice, index, .. } => {
            collect_value_use(slice, used);
            collect_value_use(index, used);
        }
    }
}

fn collect_value_use(value: &MirValue, used: &mut HashSet<String>) {
    if let Some(name) = temp_name(value) {
        used.insert(name.to_string());
    }
}

fn run_cfg_simplify(module: &mut MirModule, context: &MirPassContext) -> MirPassResult {
    let mut changed = false;
    for function in &mut module.functions {
        if context.opt_level >= 2 {
            changed |= simplify_constant_branches(function);
            changed |= simplify_jump_targets(function);
        }
        changed |= remove_unreachable_blocks(function);
        if context.opt_level >= 2 {
            changed |= simplify_jump_targets(function);
            changed |= remove_unreachable_blocks(function);
        }
    }
    MirPassResult {
        changed,
        diagnostics: Vec::new(),
    }
}

fn simplify_constant_branches(function: &mut MirFunction) -> bool {
    let constants = collect_const_bool_temps(function);
    let mut changed = false;
    for block in &mut function.blocks {
        let MirTerminator::Branch {
            condition,
            then_label,
            else_label,
        } = &block.terminator
        else {
            continue;
        };
        let Some(condition) = get_known_bool(condition, &constants) else {
            continue;
        };
        block.terminator = MirTerminator::Jump {
            label: if condition {
                then_label.clone()
            } else {
                else_label.clone()
            },
        };
        changed = true;
    }
    changed
}

fn simplify_jump_targets(function: &mut MirFunction) -> bool {
    let mut changed = false;
    let blocks_by_label = function
        .blocks
        .iter()
        .map(|block| (block.label.clone(), block.clone()))
        .collect::<HashMap<_, _>>();

    for block in &mut function.blocks {
        match &block.terminator {
            MirTerminator::Jump { label } => {
                let resolved = resolve_empty_jump_target(label, &blocks_by_label);
                if resolved != *label {
                    block.terminator = MirTerminator::Jump { label: resolved };
                    changed = true;
                }
            }
            MirTerminator::Branch {
                condition,
                then_label,
                else_label,
            } => {
                let resolved_then = resolve_empty_jump_target(then_label, &blocks_by_label);
                let resolved_else = resolve_empty_jump_target(else_label, &blocks_by_label);
                if resolved_then == resolved_else {
                    block.terminator = MirTerminator::Jump {
                        label: resolved_then,
                    };
                    changed = true;
                } else if resolved_then != *then_label || resolved_else != *else_label {
                    block.terminator = MirTerminator::Branch {
                        condition: condition.clone(),
                        then_label: resolved_then,
                        else_label: resolved_else,
                    };
                    changed = true;
                }
            }
            MirTerminator::Return { .. } => {}
        }
    }

    changed
}

fn remove_unreachable_blocks(function: &mut MirFunction) -> bool {
    if function.blocks.is_empty() {
        return false;
    }
    let reachable = collect_reachable_labels(function);
    let before = function.blocks.len();
    function
        .blocks
        .retain(|block| reachable.contains(&block.label));
    function.blocks.len() != before
}

fn collect_reachable_labels(function: &MirFunction) -> HashSet<String> {
    let blocks_by_label = function
        .blocks
        .iter()
        .map(|block| (block.label.clone(), block))
        .collect::<HashMap<_, _>>();
    let mut reachable = HashSet::new();
    let mut worklist = vec![function.blocks[0].label.clone()];

    while let Some(label) = worklist.pop() {
        if !reachable.insert(label.clone()) {
            continue;
        }
        let Some(block) = blocks_by_label.get(&label) else {
            continue;
        };
        for target in terminator_targets(&block.terminator) {
            if !reachable.contains(&target) {
                worklist.push(target);
            }
        }
    }
    reachable
}

fn terminator_targets(terminator: &MirTerminator) -> Vec<String> {
    match terminator {
        MirTerminator::Jump { label } => vec![label.clone()],
        MirTerminator::Branch {
            then_label,
            else_label,
            ..
        } => vec![then_label.clone(), else_label.clone()],
        MirTerminator::Return { .. } => Vec::new(),
    }
}

fn resolve_empty_jump_target(label: &str, blocks_by_label: &HashMap<String, MirBlock>) -> String {
    let mut current = label.to_string();
    let mut seen = HashSet::new();
    while seen.insert(current.clone()) {
        let Some(block) = blocks_by_label.get(&current) else {
            return current;
        };
        let MirTerminator::Jump { label } = &block.terminator else {
            return current;
        };
        if !block.instructions.is_empty() {
            return current;
        }
        current.clone_from(label);
    }
    label.to_string()
}

fn collect_const_bool_temps(function: &MirFunction) -> HashMap<String, bool> {
    let mut constants = HashMap::new();
    for block in &function.blocks {
        for instruction in &block.instructions {
            if let MirInstruction::ConstBool { target, value } = instruction
                && let Some(name) = temp_name(target)
            {
                constants.insert(name.to_string(), *value);
            }
        }
    }
    constants
}

fn get_known_bool(value: &MirValue, constants: &HashMap<String, bool>) -> Option<bool> {
    match value {
        MirValue::ConstBool { value, .. } => Some(*value),
        MirValue::Temp { name, .. } => constants.get(name).copied(),
        MirValue::Param { .. }
        | MirValue::Local { .. }
        | MirValue::ConstInt { .. }
        | MirValue::ConstFloat { .. } => None,
    }
}

fn instruction_target(instruction: &MirInstruction) -> Option<&MirValue> {
    match instruction {
        MirInstruction::ConstInt { target, .. }
        | MirInstruction::ConstFloat { target, .. }
        | MirInstruction::ConstBool { target, .. }
        | MirInstruction::Move { target, .. }
        | MirInstruction::Binary { target, .. }
        | MirInstruction::Unary { target, .. }
        | MirInstruction::Compare { target, .. }
        | MirInstruction::Cast { target, .. }
        | MirInstruction::Address { target, .. }
        | MirInstruction::Load { target, .. }
        | MirInstruction::MakeSlice { target, .. }
        | MirInstruction::SliceData { target, .. }
        | MirInstruction::SliceLen { target, .. }
        | MirInstruction::Subslice { target, .. } => Some(target),
        MirInstruction::Call { target, .. } => target.as_ref(),
        MirInstruction::Store { .. } => None,
    }
}

fn value_type(value: &MirValue) -> &MirType {
    match value {
        MirValue::Param { type_node, .. }
        | MirValue::Local { type_node, .. }
        | MirValue::Temp { type_node, .. }
        | MirValue::ConstInt { type_node, .. }
        | MirValue::ConstFloat { type_node, .. }
        | MirValue::ConstBool { type_node, .. } => type_node,
    }
}

fn place_type(place: &MirPlace) -> &MirType {
    match place {
        MirPlace::Param { type_node, .. }
        | MirPlace::Local { type_node, .. }
        | MirPlace::Deref { type_node, .. }
        | MirPlace::Index { type_node, .. }
        | MirPlace::SliceIndex { type_node, .. }
        | MirPlace::Field { type_node, .. } => type_node,
    }
}

fn temp_name(value: &MirValue) -> Option<&str> {
    match value {
        MirValue::Temp { name, .. } => Some(name),
        MirValue::Param { .. }
        | MirValue::Local { .. }
        | MirValue::ConstInt { .. }
        | MirValue::ConstFloat { .. }
        | MirValue::ConstBool { .. } => None,
    }
}

fn is_integer_type(type_node: &MirType) -> bool {
    matches!(
        type_node,
        MirType::Primitive(
            MirPrimitiveTypeName::I32
                | MirPrimitiveTypeName::I64
                | MirPrimitiveTypeName::U32
                | MirPrimitiveTypeName::U64
        )
    )
}

fn is_f64_type(type_node: &MirType) -> bool {
    matches!(type_node, MirType::Primitive(MirPrimitiveTypeName::F64))
}

fn contains_slice_type(type_node: &MirType) -> bool {
    match type_node {
        MirType::Slice(_) => true,
        MirType::Pointer(element_type) => contains_slice_type(element_type),
        MirType::Primitive(_) | MirType::Struct(_) | MirType::Void => false,
    }
}

fn is_signed_integer_type(type_node: &MirType) -> bool {
    matches!(
        type_node,
        MirType::Primitive(MirPrimitiveTypeName::I32 | MirPrimitiveTypeName::I64)
    )
}

fn integer_min(type_node: &MirType) -> Option<i128> {
    match type_node {
        MirType::Primitive(MirPrimitiveTypeName::I32) => Some(-(1_i128 << 31)),
        MirType::Primitive(MirPrimitiveTypeName::I64) => Some(-(1_i128 << 63)),
        MirType::Primitive(MirPrimitiveTypeName::U32 | MirPrimitiveTypeName::U64) => Some(0),
        MirType::Primitive(MirPrimitiveTypeName::F64 | MirPrimitiveTypeName::Bool)
        | MirType::Pointer(_)
        | MirType::Slice(_)
        | MirType::Struct(_)
        | MirType::Void => None,
    }
}

fn integer_max(type_node: &MirType) -> Option<i128> {
    match type_node {
        MirType::Primitive(MirPrimitiveTypeName::I32) => Some((1_i128 << 31) - 1),
        MirType::Primitive(MirPrimitiveTypeName::I64) => Some((1_i128 << 63) - 1),
        MirType::Primitive(MirPrimitiveTypeName::U32) => Some((1_i128 << 32) - 1),
        MirType::Primitive(MirPrimitiveTypeName::U64) => Some((1_i128 << 64) - 1),
        MirType::Primitive(MirPrimitiveTypeName::F64 | MirPrimitiveTypeName::Bool)
        | MirType::Pointer(_)
        | MirType::Slice(_)
        | MirType::Struct(_)
        | MirType::Void => None,
    }
}

fn fits_integer_type(value: i128, type_node: &MirType) -> bool {
    is_integer_type(type_node)
        && integer_min(type_node).is_some_and(|min| value >= min)
        && integer_max(type_node).is_some_and(|max| value <= max)
}
