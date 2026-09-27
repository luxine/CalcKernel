use std::collections::HashMap;

use crate::{
    EffectAccess, EffectCall, EffectFunction, EffectGraph, EffectSolveConfig, EffectSolveResult,
    EffectSummary, EffectTarget, MemoryEffect, solve_effect_graph,
};

use super::{
    CalcKernelType, CheckedProgram, CompilerBuiltinEffect, ContractEffectKind, Expression,
    FunctionInfo, SourceSpan, Statement, get_compiler_builtin,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SourceRoot {
    External(EffectTarget),
    Private,
    All,
}

impl SourceRoot {
    const fn effect_target(self) -> Option<EffectTarget> {
        match self {
            Self::External(target) => Some(target),
            Self::Private => None,
            Self::All => Some(EffectTarget::All),
        }
    }
}

struct SourceEffectCollector<'program> {
    program: &'program CheckedProgram,
    roots: HashMap<String, SourceRoot>,
    direct: EffectSummary,
    calls: Vec<EffectCall>,
}

impl<'program> SourceEffectCollector<'program> {
    fn new(program: &'program CheckedProgram, function: &FunctionInfo) -> Self {
        let roots = function
            .params
            .iter()
            .enumerate()
            .map(|(index, param)| {
                let root = match param.type_node {
                    CalcKernelType::Slice(_) => u32::try_from(index)
                        .map(EffectTarget::Parameter)
                        .map(SourceRoot::External)
                        .unwrap_or(SourceRoot::All),
                    CalcKernelType::Pointer(_) => SourceRoot::All,
                    _ => SourceRoot::Private,
                };
                (param.name.clone(), root)
            })
            .collect();
        Self {
            program,
            roots,
            direct: EffectSummary::empty(),
            calls: Vec::new(),
        }
    }

    fn collect(mut self, function: &FunctionInfo) -> EffectFunction {
        self.scan_statements(&function.declaration.body.statements);
        EffectFunction {
            name: function.name.clone(),
            parameter_count: u32::try_from(function.params.len()).unwrap_or(u32::MAX),
            direct: self.direct,
            calls: self.calls,
        }
    }

    fn scan_statements(&mut self, statements: &[Statement]) {
        for statement in statements {
            match statement {
                Statement::Block(block) => self.scan_nested_block(&block.statements),
                Statement::Unsafe(statement) => self.scan_nested_block(&statement.block.statements),
                Statement::Let(statement) => {
                    self.scan_expression(&statement.initializer);
                    self.roots.insert(
                        statement.name.name.clone(),
                        self.root_of_expression(&statement.initializer),
                    );
                }
                Statement::Assignment(statement) => {
                    self.scan_expression(&statement.value);
                    self.scan_write_place(&statement.target);
                    if let Expression::Identifier { name, .. } = &statement.target {
                        self.roots
                            .insert(name.clone(), self.root_of_expression(&statement.value));
                    }
                }
                Statement::Call(statement) => self.scan_expression(&statement.call),
                Statement::Return(statement) => {
                    if let Some(value) = &statement.value {
                        self.scan_expression(value);
                    }
                }
                Statement::If(statement) => {
                    self.scan_expression(&statement.condition);
                    let before = self.roots.clone();
                    self.scan_statements(&statement.then_block.statements);
                    let then_roots = self.roots.clone();
                    self.roots = before.clone();
                    if let Some(else_block) = &statement.else_block {
                        self.scan_statements(&else_block.statements);
                    }
                    self.roots = merge_roots(&before, &then_roots, &self.roots);
                }
                Statement::While(statement) => {
                    self.scan_expression(&statement.condition);
                    let before = self.roots.clone();
                    self.scan_statements(&statement.body.statements);
                    self.roots = merge_roots(&before, &before, &self.roots);
                }
                Statement::Break(_) | Statement::Continue(_) | Statement::Error { .. } => {}
            }
        }
    }

    fn scan_nested_block(&mut self, statements: &[Statement]) {
        let outer = self.roots.clone();
        self.scan_statements(statements);
        self.roots.retain(|name, _| outer.contains_key(name));
        for (name, root) in outer {
            self.roots.entry(name).or_insert(root);
        }
    }

    fn scan_expression(&mut self, expression: &Expression) {
        match expression {
            Expression::Identifier { .. }
            | Expression::IntegerLiteral { .. }
            | Expression::FloatLiteral { .. }
            | Expression::BoolLiteral { .. }
            | Expression::Error { .. } => {}
            Expression::Parenthesized { expression, .. } => self.scan_expression(expression),
            Expression::Unary {
                operator, operand, ..
            } => {
                self.scan_expression(operand);
                if operator == "-" {
                    self.direct.may_fail = true;
                }
            }
            Expression::Binary {
                operator,
                left,
                right,
                ..
            } => {
                self.scan_expression(left);
                self.scan_expression(right);
                if matches!(operator.as_str(), "+" | "-" | "*" | "/" | "%") {
                    self.direct.may_fail = true;
                }
            }
            Expression::Call { callee, args, .. } => {
                for arg in args {
                    self.scan_expression(arg);
                }
                let Expression::Identifier { name, .. } = callee.as_ref() else {
                    self.direct.join(&EffectSummary::full_conservative());
                    return;
                };
                if let Some(builtin) = get_compiler_builtin(name) {
                    if builtin.effect == CompilerBuiltinEffect::ObservableOutput {
                        self.direct.runtime_effect = true;
                    }
                    return;
                }
                let is_unsafe = self
                    .program
                    .function_map
                    .get(name)
                    .is_some_and(|function| function.is_unsafe);
                self.calls.push(EffectCall {
                    callee: name.clone(),
                    arguments: args
                        .iter()
                        .map(|arg| self.root_of_expression(arg).effect_target())
                        .collect(),
                    is_unsafe,
                });
                self.direct.unsafe_calls |= is_unsafe;
            }
            Expression::SliceConstructor { data, len, .. } => {
                self.scan_expression(data);
                self.scan_expression(len);
            }
            Expression::Field { object, .. } => self.scan_expression(object),
            Expression::Index { object, index, .. } => {
                self.scan_expression(object);
                self.scan_expression(index);
                self.add_memory_effect(self.root_of_expression(object), MemoryEffect::Read);
                self.direct.may_fail = true;
            }
            Expression::Subslice {
                slice, start, end, ..
            } => {
                self.scan_expression(slice);
                self.scan_expression(start);
                self.scan_expression(end);
                self.direct.may_fail = true;
            }
        }
    }

    fn scan_write_place(&mut self, expression: &Expression) {
        match expression {
            Expression::Identifier { .. } => {}
            Expression::Parenthesized { expression, .. } => self.scan_write_place(expression),
            Expression::Index { object, index, .. } => {
                self.scan_expression(object);
                self.scan_expression(index);
                self.add_memory_effect(self.root_of_expression(object), MemoryEffect::Write);
                self.direct.may_fail = true;
            }
            Expression::Field { object, .. } => {
                self.scan_expression(object);
                self.add_memory_effect(self.root_of_expression(object), MemoryEffect::Write);
            }
            _ => self.scan_expression(expression),
        }
    }

    fn add_memory_effect(&mut self, root: SourceRoot, effect: MemoryEffect) {
        if let Some(target) = root.effect_target() {
            self.direct.add_access(EffectAccess { target, effect });
        }
    }

    fn root_of_expression(&self, expression: &Expression) -> SourceRoot {
        match expression {
            Expression::Identifier { name, .. } => {
                self.roots.get(name).copied().unwrap_or(SourceRoot::Private)
            }
            Expression::Parenthesized { expression, .. } => self.root_of_expression(expression),
            Expression::Subslice { slice, .. } => self.root_of_expression(slice),
            Expression::SliceConstructor { data, .. } => self.root_of_expression(data),
            Expression::Field { object, .. } => self.root_of_expression(object),
            Expression::Call { .. } => SourceRoot::All,
            Expression::Index { object, .. } => self.root_of_expression(object),
            Expression::IntegerLiteral { .. }
            | Expression::FloatLiteral { .. }
            | Expression::BoolLiteral { .. }
            | Expression::Unary { .. }
            | Expression::Binary { .. }
            | Expression::Error { .. } => SourceRoot::Private,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectCeilingViolation {
    pub span: SourceSpan,
    pub message: String,
}

#[must_use]
pub fn analyze_checked_program_effects(
    program: &CheckedProgram,
    config: EffectSolveConfig,
) -> EffectSolveResult {
    let graph = EffectGraph {
        functions: program
            .functions
            .iter()
            .map(|function| SourceEffectCollector::new(program, function).collect(function))
            .collect(),
    };
    solve_effect_graph(&graph, config)
}

#[must_use]
pub fn validate_effect_ceilings(program: &CheckedProgram) -> Vec<EffectCeilingViolation> {
    let mut violations = Vec::new();
    for function in &program.functions {
        let Some(contract) = &function.contract else {
            continue;
        };
        let Some(ceiling) = &contract.effects else {
            continue;
        };
        let Some(summary) = program.effect_summaries.get(&function.name) else {
            continue;
        };
        let span = function
            .declaration
            .contract
            .as_ref()
            .and_then(|contract| contract.effects.as_ref())
            .map_or(function.declaration.name.span, |effects| effects.span);
        if let Some(access) = summary
            .accesses()
            .find(|access| !ceiling_allows(function, ceiling, *access))
        {
            violations.push(EffectCeilingViolation {
                span,
                message: ceiling_violation_message(function, access),
            });
        }
    }
    violations
}

fn ceiling_allows(
    function: &FunctionInfo,
    ceiling: &super::CheckedContractEffectCeiling,
    access: EffectAccess,
) -> bool {
    let EffectTarget::Parameter(index) = access.target else {
        return false;
    };
    let Some(param) = function.params.get(index as usize) else {
        return false;
    };
    let declared = ceiling
        .items
        .iter()
        .find(|(name, _)| name == &param.name)
        .map_or(MemoryEffect::None, |(_, effect)| memory_effect(*effect));
    declared.allows(access.effect)
}

fn memory_effect(effect: ContractEffectKind) -> MemoryEffect {
    match effect {
        ContractEffectKind::None => MemoryEffect::None,
        ContractEffectKind::Read => MemoryEffect::Read,
        ContractEffectKind::Write => MemoryEffect::Write,
        ContractEffectKind::ReadWrite => MemoryEffect::ReadWrite,
    }
}

fn ceiling_violation_message(function: &FunctionInfo, access: EffectAccess) -> String {
    let effect = match access.effect {
        MemoryEffect::None => "none",
        MemoryEffect::Read => "read",
        MemoryEffect::Write => "write",
        MemoryEffect::ReadWrite => "readwrite",
    };
    match access.target {
        EffectTarget::Parameter(index) => {
            let name = function
                .params
                .get(index as usize)
                .map_or("<unknown>", |param| param.name.as_str());
            format!(
                "Effect ceiling for function '{}' does not allow {effect} access to slice parameter '{name}'.",
                function.name
            )
        }
        EffectTarget::All => format!(
            "Effect ceiling for function '{}' cannot cover conservative {effect} access to externally reachable memory.",
            function.name
        ),
    }
}

fn merge_roots(
    base: &HashMap<String, SourceRoot>,
    left: &HashMap<String, SourceRoot>,
    right: &HashMap<String, SourceRoot>,
) -> HashMap<String, SourceRoot> {
    base.iter()
        .map(|(name, base_root)| {
            let left = left.get(name).copied().unwrap_or(*base_root);
            let right = right.get(name).copied().unwrap_or(*base_root);
            (
                name.clone(),
                if left == right { left } else { SourceRoot::All },
            )
        })
        .collect()
}
