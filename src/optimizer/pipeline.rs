use crate::{MirModule, MirValidationError, validate_mir_module};

use super::passes::*;

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
