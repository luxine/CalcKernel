use num_bigint::BigInt;

use crate::{ContractFactAffineExpression, ContractFactAffineTerm, ContractFactPredicate, ValueId};

use super::{IntegerType, ScalarInterval};

/// Mathematical interpretation of normalized single-value contract comparisons.
/// This does not establish scope or provenance; callers must validate those separately.
pub(crate) fn contract_scalar_interval<'a>(
    predicates: impl IntoIterator<Item = &'a ContractFactPredicate>,
    value: ValueId,
    ty: IntegerType,
) -> Option<ScalarInterval> {
    let mut lower = BigInt::from(ty.minimum_i128());
    let mut upper = BigInt::from(ty.maximum_i128());
    let mut constrained = false;
    for predicate in predicates {
        let ContractFactPredicate::Comparison {
            operator,
            left,
            right,
        } = predicate
        else {
            continue;
        };
        let bound = if let (Some(offset), Some(constant)) =
            (value_offset(left, value), constant(right))
        {
            Some((operator.as_str(), constant - offset))
        } else if let (Some(constant), Some(offset)) = (constant(left), value_offset(right, value))
        {
            reverse(operator).map(|operator| (operator, constant - offset))
        } else {
            None
        };
        let Some((operator, bound)) = bound else {
            continue;
        };
        match operator {
            "<" => upper = upper.min(bound - 1),
            "<=" => upper = upper.min(bound),
            ">" => lower = lower.max(bound + 1),
            ">=" => lower = lower.max(bound),
            "==" => {
                lower = lower.max(bound.clone());
                upper = upper.min(bound);
            }
            _ => continue,
        }
        constrained = true;
    }
    constrained
        .then(|| ScalarInterval::new(lower, upper).ok())
        .flatten()
}

fn value_offset(expression: &ContractFactAffineExpression, value: ValueId) -> Option<BigInt> {
    let [term] = expression.terms.as_slice() else {
        return None;
    };
    (term.term == ContractFactAffineTerm::Value(value) && term.coefficient == BigInt::from(1))
        .then(|| expression.constant.clone())
}

fn constant(expression: &ContractFactAffineExpression) -> Option<BigInt> {
    expression
        .terms
        .is_empty()
        .then(|| expression.constant.clone())
}

fn reverse(operator: &str) -> Option<&'static str> {
    match operator {
        "<" => Some(">"),
        "<=" => Some(">="),
        ">" => Some("<"),
        ">=" => Some("<="),
        "==" => Some("=="),
        _ => None,
    }
}
