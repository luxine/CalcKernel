use crate::{
    CheckedAffineExpression, CheckedAffineTerm, CheckedContractPredicate, CheckedProgram,
    ContractEffectKind, KirModule,
};

use super::c::emit_c_kir_header_with_mode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeHeaderMode {
    Dynamic,
    StaticOrObject,
}

/// Emits the authoritative Native C ABI header for a library artifact.
#[must_use]
pub fn emit_native_header(module: &KirModule, mode: NativeHeaderMode) -> String {
    emit_c_kir_header_with_mode(module, mode == NativeHeaderMode::Dynamic)
}

/// Prepends deterministic, ABI-neutral comments for exported unsafe contracts.
///
/// Slice parameters use their flattened C ABI spellings (`name_data` and
/// `name_len`) so foreign callers can mechanically map every obligation to the
/// generated declaration. The input header bytes are otherwise unchanged.
#[must_use]
pub fn annotate_unsafe_contracts(header: &str, program: &CheckedProgram) -> String {
    let mut comments = String::new();
    for function in &program.functions {
        if !function.exported || !function.is_unsafe {
            continue;
        }
        let Some(contract) = &function.contract else {
            continue;
        };
        comments.push_str(&format!("/* CK unsafe {}\n", function.name));
        for predicate in &contract.predicates {
            for predicate in flatten_predicate(predicate) {
                comments.push_str(" * requires ");
                comments.push_str(&format_predicate(predicate));
                comments.push('\n');
            }
        }
        if let Some(effects) = &contract.effects {
            comments.push_str(" * effects ");
            if effects.is_none {
                comments.push_str("none");
            } else {
                comments.push_str(
                    &effects
                        .items
                        .iter()
                        .map(|(name, effect)| {
                            format!(
                                "{}({name}_data[0..{name}_len])",
                                match effect {
                                    ContractEffectKind::None => "none",
                                    ContractEffectKind::Read => "read",
                                    ContractEffectKind::Write => "write",
                                    ContractEffectKind::ReadWrite => "readwrite",
                                }
                            )
                        })
                        .collect::<Vec<_>>()
                        .join(", "),
                );
            }
            comments.push('\n');
        }
        comments.push_str(" */\n");
    }
    comments.push_str(header);
    comments
}

fn flatten_predicate(predicate: &CheckedContractPredicate) -> Vec<&CheckedContractPredicate> {
    match predicate {
        CheckedContractPredicate::Conjunction(items) => {
            items.iter().flat_map(flatten_predicate).collect()
        }
        predicate => vec![predicate],
    }
}

fn format_predicate(predicate: &CheckedContractPredicate) -> String {
    match predicate {
        CheckedContractPredicate::Comparison {
            operator,
            left,
            right,
        } => format!(
            "{} {operator} {}",
            format_affine(left),
            format_affine(right)
        ),
        CheckedContractPredicate::MultipleOf { value, modulus } => {
            format!("multiple_of({}, {modulus})", format_affine(value))
        }
        CheckedContractPredicate::NoAlias { left, right } => {
            format!("noalias({left}_data[0..{left}_len], {right}_data[0..{right}_len])")
        }
        CheckedContractPredicate::Aligned { pointer, alignment } => format!(
            "aligned({}, {alignment})",
            match pointer {
                crate::CheckedContractPointer::Parameter(name) => name.clone(),
                crate::CheckedContractPointer::SliceData(name) => format!("{name}_data"),
            }
        ),
        CheckedContractPredicate::Conjunction(_) => {
            unreachable!("conjunctions are flattened before formatting")
        }
    }
}

fn format_affine(expression: &CheckedAffineExpression) -> String {
    let mut output = String::new();
    for term in &expression.terms {
        let name = match &term.term {
            CheckedAffineTerm::Parameter(name) => name.clone(),
            CheckedAffineTerm::SliceLength(name) => format!("{name}_len"),
        };
        push_signed_component(&mut output, &term.coefficient, &name);
    }
    if expression.constant != "0" || output.is_empty() {
        push_signed_component(&mut output, &expression.constant, "");
    }
    output
}

fn push_signed_component(output: &mut String, coefficient: &str, name: &str) {
    let (negative, magnitude) = coefficient
        .strip_prefix('-')
        .map_or((false, coefficient), |value| (true, value));
    if magnitude == "0" {
        return;
    }
    if output.is_empty() {
        if negative {
            output.push('-');
        }
    } else if negative {
        output.push_str(" - ");
    } else {
        output.push_str(" + ");
    }
    if name.is_empty() {
        output.push_str(magnitude);
    } else if magnitude == "1" {
        output.push_str(name);
    } else {
        output.push_str(magnitude);
        output.push_str(" * ");
        output.push_str(name);
    }
}
