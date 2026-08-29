use num_bigint::BigInt;

use crate::{
    FactId, InstructionId, KirArithmeticSemantics, KirCheckConditionKind, KirFunction,
    KirInstruction, KirInstructionKind, KirModule, KirTerminator, MirBinaryOp, MirUnaryOp, ProofId,
    ValueId, compute_kir_dominators,
};

use super::{
    ContractFactAffineExpression, ContractFactAffineTerm, ContractFactPredicate, ContractFactSet,
    FactArena, FactDerivation, FactOrigin, FactPredicate, ProofArena, ProofCertificate, ProofStep,
    ScalarAnalysisBudget, ScalarAnalysisResult, ScalarClaim, ScalarFailure, ScalarValue,
    contract_fact_dominates_at, refine_scalar_comparison, scalar_binary,
};

/// A deterministic compiler-internal evidence validation error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceValidationError {
    pub message: String,
    pub fact: Option<FactId>,
    pub proof: Option<ProofId>,
    pub step: Option<u32>,
}

/// Complete result: callers must reject artifact production when errors is non-empty.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EvidenceValidationResult {
    pub errors: Vec<EvidenceValidationError>,
}

#[must_use]
pub fn verify_fact_arena(
    module: &KirModule,
    contracts: Option<&ContractFactSet>,
    facts: &FactArena,
    generation: u32,
) -> EvidenceValidationResult {
    let mut errors = Vec::new();
    if facts.generation() != generation {
        errors.push(fact_error(
            None,
            format!(
                "fact arena belongs to stale generation {}, expected {generation}",
                facts.generation()
            ),
        ));
    }
    for (index, fact) in facts.facts().iter().enumerate() {
        if fact.id.index() as usize != index {
            errors.push(fact_error(
                Some(fact.id),
                "fact identity does not match arena order",
            ));
            continue;
        }
        if fact.generation != generation {
            errors.push(fact_error(
                Some(fact.id),
                format!(
                    "fact{} belongs to stale generation {}, expected {generation}",
                    fact.id.index(),
                    fact.generation
                ),
            ));
            continue;
        }
        match (&fact.origin, &fact.derivation) {
            (FactOrigin::Proven, FactDerivation::TrustedContractLeaf) => {
                errors.push(fact_error(
                    Some(fact.id),
                    format!(
                        "fact{} proven origin cannot use a trusted-contract derivation",
                        fact.id.index()
                    ),
                ));
                continue;
            }
            (FactOrigin::TrustedContract { .. }, FactDerivation::TrustedContractLeaf) => {}
            (FactOrigin::TrustedContract { .. }, _) => {
                errors.push(fact_error(
                    Some(fact.id),
                    format!(
                        "fact{} trusted-contract origin cannot use a compiler derivation",
                        fact.id.index()
                    ),
                ));
                continue;
            }
            (FactOrigin::Proven, _) => {}
        }
        if let FactOrigin::TrustedContract { instance } = fact.origin {
            let Some(contracts) = contracts else {
                errors.push(fact_error(
                    Some(fact.id),
                    format!("fact{} has no contract import table", fact.id.index()),
                ));
                continue;
            };
            let Some(record) = contracts
                .instances()
                .iter()
                .find(|record| record.id == instance)
            else {
                errors.push(fact_error(
                    Some(fact.id),
                    format!(
                        "fact{} names missing contract instance ci{}",
                        fact.id.index(),
                        instance.index()
                    ),
                ));
                continue;
            };
            if !record.facts.contains(&fact.id) {
                errors.push(fact_error(
                    Some(fact.id),
                    format!(
                        "fact{} is not owned by contract instance ci{}",
                        fact.id.index(),
                        instance.index()
                    ),
                ));
                continue;
            }
            if !trusted_fact_scope_matches(record, instance, &fact.scope) {
                errors.push(fact_error(
                    Some(fact.id),
                    format!(
                        "fact{} scope does not match contract instance ci{} source",
                        fact.id.index(),
                        instance.index()
                    ),
                ));
                continue;
            }
            if let Some(expected) = contracts.facts().get(fact.id)
                && (expected.scope != fact.scope || expected.predicate != fact.predicate)
            {
                errors.push(fact_error(
                    Some(fact.id),
                    format!(
                        "fact{} contract substitution or scope is invalid",
                        fact.id.index()
                    ),
                ));
            }
        } else {
            verify_proven_fact(module, facts, fact, &mut errors);
        }
    }
    EvidenceValidationResult { errors }
}

fn trusted_fact_scope_matches(
    record: &super::ContractFactInstance,
    instance: super::ContractInstanceId,
    scope: &super::FactScope,
) -> bool {
    match (&record.source, scope) {
        (
            super::ContractInstanceSource::FunctionEntry,
            super::FactScope::FunctionEntry(function),
        ) => *function == record.callee,
        (
            super::ContractInstanceSource::Call { .. },
            super::FactScope::CalleeInstance {
                instance: scoped,
                callee,
            },
        ) => *scoped == instance && *callee == record.callee,
        (
            super::ContractInstanceSource::InlineClone {
                function, clone, ..
            },
            super::FactScope::InlineClone {
                function: scoped_function,
                clone: scoped_clone,
                blocks,
            },
        ) => {
            *function == *scoped_function
                && *clone == *scoped_clone
                && !blocks.is_empty()
                && blocks.windows(2).all(|pair| pair[0] < pair[1])
        }
        _ => false,
    }
}

#[must_use]
pub fn verify_proof_arena(
    module: &KirModule,
    facts: &FactArena,
    contracts: Option<&ContractFactSet>,
    proofs: &ProofArena,
    generation: u32,
) -> EvidenceValidationResult {
    let mut errors = verify_fact_arena(module, contracts, facts, generation).errors;
    if proofs.generation() != generation {
        errors.push(proof_error(
            None,
            None,
            format!(
                "proof arena belongs to stale generation {}, expected {generation}",
                proofs.generation()
            ),
        ));
    }
    for (index, proof) in proofs.proofs().iter().enumerate() {
        if proof.id.index() as usize != index {
            errors.push(proof_error(
                Some(proof.id),
                None,
                "proof identity does not match arena order",
            ));
            continue;
        }
        if proof.generation != generation {
            errors.push(proof_error(
                Some(proof.id),
                None,
                format!(
                    "proof{} belongs to stale generation {}, expected {generation}",
                    proof.id.index(),
                    proof.generation
                ),
            ));
            continue;
        }
        verify_certificate(module, facts, contracts, proof, &mut errors);
    }
    EvidenceValidationResult { errors }
}

#[must_use]
pub fn verify_scalar_analysis_result(
    function: &KirFunction,
    analysis: &ScalarAnalysisResult,
) -> EvidenceValidationResult {
    let mut errors = Vec::new();
    if analysis.function() != function.id {
        errors.push(EvidenceValidationError {
            message: "scalar analysis belongs to a different KIR function".to_string(),
            fact: None,
            proof: None,
            step: None,
        });
    }
    let expected = ScalarAnalysisBudget::for_function(function, analysis.config());
    if analysis.budget() != expected {
        errors.push(EvidenceValidationError {
            message: "scalar analysis budget identity does not match current KIR".to_string(),
            fact: None,
            proof: None,
            step: None,
        });
    }
    if analysis.steps() > analysis.budget().max_steps() {
        errors.push(EvidenceValidationError {
            message: "scalar analysis exceeded its deterministic step budget".to_string(),
            fact: None,
            proof: None,
            step: None,
        });
    }
    if !analysis.exhausted()
        && analysis.narrowing_iterations_run() != analysis.budget().narrowing_iterations()
    {
        errors.push(EvidenceValidationError {
            message: "scalar analysis did not execute its fixed narrowing schedule".to_string(),
            fact: None,
            proof: None,
            step: None,
        });
    }
    EvidenceValidationResult { errors }
}

fn verify_proven_fact(
    module: &KirModule,
    facts: &FactArena,
    fact: &super::Fact,
    errors: &mut Vec<EvidenceValidationError>,
) {
    for dependency in fact_dependencies(&fact.derivation) {
        if dependency.index() >= fact.id.index() || facts.get(dependency).is_none() {
            errors.push(fact_error(
                Some(fact.id),
                format!(
                    "fact{} has invalid dependency fact{}",
                    fact.id.index(),
                    dependency.index()
                ),
            ));
            return;
        }
    }
    match &fact.derivation {
        FactDerivation::Constant { instruction } => {
            let Some((_, _, instruction)) = find_instruction(module, *instruction) else {
                errors.push(fact_error(
                    Some(fact.id),
                    format!(
                        "fact{} names missing constant instruction i{}",
                        fact.id.index(),
                        instruction.index()
                    ),
                ));
                return;
            };
            if !constant_matches_predicate(instruction, &fact.predicate) {
                errors.push(fact_error(
                    Some(fact.id),
                    format!("fact{} constant derivation is invalid", fact.id.index()),
                ));
            }
        }
        FactDerivation::BinaryTransfer {
            instruction,
            inputs,
        } => {
            if !binary_fact_matches(module, facts, *instruction, inputs, &fact.predicate) {
                errors.push(fact_error(
                    Some(fact.id),
                    format!("fact{} binary transfer is invalid", fact.id.index()),
                ));
            }
        }
        FactDerivation::BranchRefinement { .. } | FactDerivation::LoopInvariant { .. } => {
            // Closed loop/branch certificates are verified in ProofArena before use by a pass.
        }
        FactDerivation::TrustedContractLeaf => {}
    }
}

#[derive(Debug, Clone)]
enum CheckedStep {
    Scalar(ScalarClaim),
    Fact(FactPredicate),
    GuardSafety,
}

fn verify_certificate(
    module: &KirModule,
    facts: &FactArena,
    contracts: Option<&ContractFactSet>,
    proof: &ProofCertificate,
    errors: &mut Vec<EvidenceValidationError>,
) {
    let Some(function) = module
        .functions
        .iter()
        .find(|function| function.id == proof.use_site.function)
    else {
        errors.push(proof_error(
            Some(proof.id),
            None,
            format!("proof{} use function is missing", proof.id.index()),
        ));
        return;
    };
    if function
        .blocks
        .iter()
        .all(|block| block.id != proof.use_site.block)
    {
        errors.push(proof_error(
            Some(proof.id),
            None,
            format!("proof{} use block is missing", proof.id.index()),
        ));
        return;
    }
    let mut checked = Vec::new();
    for (index, step) in proof.steps.iter().enumerate() {
        match check_step(module, function, facts, contracts, proof, step, &checked) {
            Ok(value) => checked.push(value),
            Err(message) => {
                errors.push(proof_error(Some(proof.id), Some(index as u32), message));
                return;
            }
        }
    }
    if checked.get(proof.root.index() as usize).is_none() {
        errors.push(proof_error(
            Some(proof.id),
            None,
            format!("proof{} root is missing", proof.id.index()),
        ));
    }
}

fn check_step(
    module: &KirModule,
    function: &KirFunction,
    facts: &FactArena,
    contracts: Option<&ContractFactSet>,
    proof: &ProofCertificate,
    step: &ProofStep,
    checked: &[CheckedStep],
) -> Result<CheckedStep, String> {
    let prefix = |suffix: &str| format!("proof{} step{} {suffix}", proof.id.index(), checked.len());
    match step {
        ProofStep::FactLeaf { fact } => {
            let Some(fact_value) = facts.get(*fact) else {
                return Err(prefix(&format!("names missing fact{}", fact.index())));
            };
            if !fact_dominates_use(module, contracts, fact_value, proof.use_site) {
                return Err(prefix("fact does not dominate the proof use"));
            }
            if matches!(fact_value.origin, FactOrigin::TrustedContract { .. }) {
                let Some(contracts) = contracts else {
                    return Err(prefix("trusted fact has no contract instance table"));
                };
                if !contract_fact_dominates_at(contracts, *fact, proof.use_site) {
                    return Err(prefix("trusted fact does not dominate the proof use"));
                }
            }
            match &fact_value.predicate {
                FactPredicate::ValueInterval { value, interval } => Ok(CheckedStep::Scalar(
                    ScalarClaim::new(*value, interval.clone(), ScalarFailure::None),
                )),
                FactPredicate::Contract(_) => Ok(CheckedStep::Fact(fact_value.predicate.clone())),
            }
        }
        ProofStep::Constant { instruction, claim } => {
            let Some((owner, _, instruction)) = find_instruction(module, *instruction) else {
                return Err(prefix("constant instruction is missing"));
            };
            if owner.id != function.id || !constant_matches_claim(instruction, claim) {
                return Err(prefix("constant claim does not match KIR instruction"));
            }
            Ok(CheckedStep::Scalar(claim.clone()))
        }
        ProofStep::BinaryTransfer {
            instruction,
            left,
            right,
            claim,
        } => {
            let left = checked_scalar(checked, left.index(), &prefix)?;
            let right = checked_scalar(checked, right.index(), &prefix)?;
            let Some((owner, _, instruction)) = find_instruction(module, *instruction) else {
                return Err(prefix("binary instruction is missing"));
            };
            if owner.id != function.id || !binary_transfer_matches(instruction, left, right, claim)
            {
                return Err(prefix("binary claim does not match local transfer"));
            }
            Ok(CheckedStep::Scalar(claim.clone()))
        }
        ProofStep::BranchRefinement {
            predecessor,
            target,
            comparison,
            left,
            right,
            taken,
            claim,
        } => {
            let left = checked_scalar(checked, left.index(), &prefix)?;
            let right = checked_scalar(checked, right.index(), &prefix)?;
            if !branch_refinement_matches(
                function,
                *predecessor,
                *target,
                *comparison,
                left,
                right,
                *taken,
                claim,
            ) {
                return Err(prefix("branch refinement is invalid"));
            }
            Ok(CheckedStep::Scalar(claim.clone()))
        }
        ProofStep::LoopInvariant {
            header,
            phi,
            transfer,
            claim,
        } => {
            if !loop_invariant_matches(function, *header, *phi, *transfer, claim) {
                return Err(prefix("loop invariant is not closed under its transfer"));
            }
            Ok(CheckedStep::Scalar(claim.clone()))
        }
        ProofStep::GuardSafety {
            condition_instruction,
            premises,
        } => {
            let premises = premises
                .iter()
                .map(|premise| {
                    checked
                        .get(premise.index() as usize)
                        .ok_or_else(|| prefix("guard-safety premise is missing"))
                })
                .collect::<Result<Vec<_>, _>>()?;
            if !guard_safety_matches(function, *condition_instruction, &premises) {
                return Err(prefix(
                    "guard-safety claim does not follow from local KIR and premises",
                ));
            }
            Ok(CheckedStep::GuardSafety)
        }
    }
}

fn fact_dominates_use(
    module: &KirModule,
    contracts: Option<&ContractFactSet>,
    fact: &super::Fact,
    site: super::FactUseSite,
) -> bool {
    match &fact.scope {
        super::FactScope::FunctionEntry(function) => *function == site.function,
        super::FactScope::Block {
            function: scope_function,
            block,
        } => {
            if *scope_function != site.function {
                return false;
            }
            let Some(owner) = module
                .functions
                .iter()
                .find(|function| function.id == *scope_function)
            else {
                return false;
            };
            if !compute_kir_dominators(owner).dominates(*block, site.block) {
                return false;
            }
            if *block != site.block {
                return true;
            }
            let Some(use_instruction) = site.instruction else {
                return true;
            };
            let Some(block) = owner.blocks.iter().find(|candidate| candidate.id == *block) else {
                return false;
            };
            let use_index = block
                .instructions
                .iter()
                .position(|instruction| instruction.id == use_instruction);
            let definition_index = fact_definition_instruction(&fact.derivation).and_then(|id| {
                block
                    .instructions
                    .iter()
                    .position(|instruction| instruction.id == id)
            });
            match (definition_index, use_index) {
                (Some(definition), Some(used)) => definition < used,
                (None, _) => true,
                _ => false,
            }
        }
        super::FactScope::CalleeInstance { .. } | super::FactScope::InlineClone { .. } => {
            contracts.is_some_and(|contracts| contract_fact_dominates_at(contracts, fact.id, site))
        }
    }
}

fn fact_definition_instruction(derivation: &FactDerivation) -> Option<InstructionId> {
    match derivation {
        FactDerivation::Constant { instruction }
        | FactDerivation::BinaryTransfer { instruction, .. } => Some(*instruction),
        FactDerivation::BranchRefinement { comparison, .. } => Some(*comparison),
        FactDerivation::TrustedContractLeaf | FactDerivation::LoopInvariant { .. } => None,
    }
}

fn checked_scalar<'a>(
    checked: &'a [CheckedStep],
    index: u32,
    prefix: &impl Fn(&str) -> String,
) -> Result<&'a ScalarClaim, String> {
    match checked.get(index as usize) {
        Some(CheckedStep::Scalar(claim)) => Ok(claim),
        Some(CheckedStep::Fact(_) | CheckedStep::GuardSafety) => {
            Err(prefix("step dependency is not a scalar claim"))
        }
        None => Err(prefix("step dependency is missing")),
    }
}

fn guard_safety_matches(
    function: &KirFunction,
    condition_instruction: InstructionId,
    premises: &[&CheckedStep],
) -> bool {
    let Some(instruction) = function
        .blocks
        .iter()
        .flat_map(|block| &block.instructions)
        .find(|instruction| instruction.id == condition_instruction)
    else {
        return false;
    };
    match &instruction.kind {
        KirInstructionKind::Binary {
            op: op @ (MirBinaryOp::Add | MirBinaryOp::Sub | MirBinaryOp::Mul),
            left,
            right,
            semantics: KirArithmeticSemantics::Checked,
        } => {
            let Some(type_node) = instruction
                .results
                .first()
                .and_then(|result| super::IntegerType::from_mir(&result.type_node))
            else {
                return false;
            };
            let (Some(left), Some(right)) = (
                scalar_for_value(function, premises, *left, type_node),
                scalar_for_value(function, premises, *right, type_node),
            ) else {
                return false;
            };
            scalar_binary(*op, KirArithmeticSemantics::Checked, &left, &right)
                .is_ok_and(|result| result.failure() == ScalarFailure::None)
        }
        KirInstructionKind::Unary {
            op: MirUnaryOp::Neg,
            operand,
            semantics: KirArithmeticSemantics::Checked,
        } => {
            let Some(type_node) = instruction
                .results
                .first()
                .and_then(|result| super::IntegerType::from_mir(&result.type_node))
            else {
                return false;
            };
            scalar_for_value(function, premises, *operand, type_node).is_some_and(|operand| {
                type_node.is_signed()
                    && operand.interval().lower() > &BigInt::from(type_node.minimum_i128())
            })
        }
        KirInstructionKind::CheckCondition { kind, args } => match kind {
            KirCheckConditionKind::ArithmeticOverflow => false,
            KirCheckConditionKind::DivisionByZero => args
                .first()
                .and_then(|value| exact_scalar(function, premises, *value))
                .is_some_and(|value| value != BigInt::from(0)),
            KirCheckConditionKind::SignedDivisionOverflow => {
                signed_division_is_safe(function, premises, args)
            }
            KirCheckConditionKind::SliceOutOfBounds => {
                slice_index_is_safe(function, premises, args)
            }
            KirCheckConditionKind::InvalidSubslice => subslice_is_safe(function, premises, args),
        },
        _ => false,
    }
}

fn scalar_for_value(
    function: &KirFunction,
    premises: &[&CheckedStep],
    value: ValueId,
    type_node: super::IntegerType,
) -> Option<ScalarValue> {
    if let Some(claim) = premises.iter().find_map(|premise| match premise {
        CheckedStep::Scalar(claim) if claim.value == value => Some(claim),
        _ => None,
    }) {
        return ScalarValue::from_interval(type_node, claim.interval.clone())
            .ok()
            .map(|scalar| scalar.with_failure(claim.failure));
    }
    resolve_constant(function, value)
        .and_then(|constant| ScalarValue::constant(type_node, constant).ok())
}

fn exact_scalar(
    function: &KirFunction,
    premises: &[&CheckedStep],
    value: ValueId,
) -> Option<BigInt> {
    resolve_constant(function, value).or_else(|| {
        premises.iter().find_map(|premise| match premise {
            CheckedStep::Scalar(claim)
                if claim.value == value && claim.interval.lower() == claim.interval.upper() =>
            {
                Some(claim.interval.lower().clone())
            }
            _ => None,
        })
    })
}

fn signed_division_is_safe(
    function: &KirFunction,
    premises: &[&CheckedStep],
    args: &[ValueId],
) -> bool {
    let [left, right] = args else {
        return false;
    };
    let (Some(type_node), Some(left), Some(right)) = (
        value_integer_type(function, *left),
        exact_scalar(function, premises, *left),
        exact_scalar(function, premises, *right),
    ) else {
        return false;
    };
    if !type_node.is_signed() {
        return false;
    }
    left != BigInt::from(type_node.minimum_i128()) || right != BigInt::from(-1)
}

fn slice_index_is_safe(
    function: &KirFunction,
    premises: &[&CheckedStep],
    args: &[ValueId],
) -> bool {
    let [slice, index] = args else {
        return false;
    };
    if let (Some(index), Some(len)) = (
        exact_scalar(function, premises, *index),
        resolve_slice_len(function, premises, *slice),
    ) {
        return index >= BigInt::from(0) && index < len;
    }
    premises.iter().any(|premise| {
        matches!(
            premise,
            CheckedStep::Fact(FactPredicate::Contract(predicate))
                if contract_proves_value_below_slice_len(predicate, *index, *slice)
        )
    })
}

fn subslice_is_safe(function: &KirFunction, premises: &[&CheckedStep], args: &[ValueId]) -> bool {
    let [slice, start, end] = args else {
        return false;
    };
    let (Some(start), Some(end), Some(len)) = (
        exact_scalar(function, premises, *start),
        exact_scalar(function, premises, *end),
        resolve_slice_len(function, premises, *slice),
    ) else {
        return false;
    };
    start >= BigInt::from(0) && start <= end && end <= len
}

fn resolve_slice_len(
    function: &KirFunction,
    premises: &[&CheckedStep],
    slice: ValueId,
) -> Option<BigInt> {
    function
        .blocks
        .iter()
        .flat_map(|block| &block.instructions)
        .find_map(|instruction| {
            (instruction.results.first().map(|result| result.value) == Some(slice)).then_some(
                match instruction.kind {
                    KirInstructionKind::MakeSlice { len, .. } => {
                        exact_scalar(function, premises, len)
                    }
                    _ => None,
                },
            )
        })
        .flatten()
}

fn contract_proves_value_below_slice_len(
    predicate: &ContractFactPredicate,
    value: ValueId,
    slice: ValueId,
) -> bool {
    let ContractFactPredicate::Comparison {
        operator,
        left,
        right,
    } = predicate
    else {
        return false;
    };
    (operator == "<"
        && affine_is_single_term(left, ContractFactAffineTerm::Value(value))
        && affine_is_single_term(right, ContractFactAffineTerm::SliceLength(slice)))
        || (operator == ">"
            && affine_is_single_term(left, ContractFactAffineTerm::SliceLength(slice))
            && affine_is_single_term(right, ContractFactAffineTerm::Value(value)))
}

fn affine_is_single_term(
    expression: &ContractFactAffineExpression,
    expected: ContractFactAffineTerm,
) -> bool {
    expression.constant == BigInt::from(0)
        && expression.terms.len() == 1
        && expression.terms[0].term == expected
        && expression.terms[0].coefficient == BigInt::from(1)
}

fn constant_matches_claim(instruction: &KirInstruction, claim: &ScalarClaim) -> bool {
    let KirInstructionKind::ConstInt { value } = &instruction.kind else {
        return false;
    };
    let Some(result) = instruction.results.first() else {
        return false;
    };
    let (Some(type_node), Ok(value)) = (
        super::IntegerType::from_mir(&result.type_node),
        value.parse::<BigInt>(),
    ) else {
        return false;
    };
    let Ok(expected) = ScalarValue::constant(type_node, value) else {
        return false;
    };
    claim.value == result.value
        && claim.interval == *expected.interval()
        && claim.failure == ScalarFailure::None
}

fn constant_matches_predicate(instruction: &KirInstruction, predicate: &FactPredicate) -> bool {
    let FactPredicate::ValueInterval { value, interval } = predicate else {
        return false;
    };
    constant_matches_claim(
        instruction,
        &ScalarClaim::new(*value, interval.clone(), ScalarFailure::None),
    )
}

fn binary_transfer_matches(
    instruction: &KirInstruction,
    left: &ScalarClaim,
    right: &ScalarClaim,
    claim: &ScalarClaim,
) -> bool {
    let KirInstructionKind::Binary {
        op,
        left: expected_left,
        right: expected_right,
        semantics,
    } = instruction.kind
    else {
        return false;
    };
    let Some(result) = instruction.results.first() else {
        return false;
    };
    let Some(type_node) = super::IntegerType::from_mir(&result.type_node) else {
        return false;
    };
    if left.value != expected_left || right.value != expected_right || claim.value != result.value {
        return false;
    }
    let (Ok(left), Ok(right)) = (
        ScalarValue::from_interval(type_node, left.interval.clone())
            .map(|value| value.with_failure(left.failure)),
        ScalarValue::from_interval(type_node, right.interval.clone())
            .map(|value| value.with_failure(right.failure)),
    ) else {
        return false;
    };
    scalar_binary(op, semantics, &left, &right).is_ok_and(|result| {
        claim.interval == *result.interval() && claim.failure == result.failure()
    })
}

fn binary_fact_matches(
    module: &KirModule,
    facts: &FactArena,
    instruction: InstructionId,
    inputs: &[FactId],
    predicate: &FactPredicate,
) -> bool {
    if inputs.len() != 2 {
        return false;
    }
    let (Some(left), Some(right), Some((_, _, instruction))) = (
        facts.get(inputs[0]),
        facts.get(inputs[1]),
        find_instruction(module, instruction),
    ) else {
        return false;
    };
    let (
        FactPredicate::ValueInterval {
            value: left_value,
            interval: left_interval,
        },
        FactPredicate::ValueInterval {
            value: right_value,
            interval: right_interval,
        },
        FactPredicate::ValueInterval { value, interval },
    ) = (&left.predicate, &right.predicate, predicate)
    else {
        return false;
    };
    binary_transfer_matches(
        instruction,
        &ScalarClaim::new(*left_value, left_interval.clone(), ScalarFailure::None),
        &ScalarClaim::new(*right_value, right_interval.clone(), ScalarFailure::None),
        &ScalarClaim::new(*value, interval.clone(), ScalarFailure::None),
    )
}

#[allow(clippy::too_many_arguments)]
fn branch_refinement_matches(
    function: &KirFunction,
    predecessor: crate::BlockId,
    target: crate::BlockId,
    comparison: InstructionId,
    left: &ScalarClaim,
    right: &ScalarClaim,
    taken: bool,
    claim: &ScalarClaim,
) -> bool {
    let Some(block) = function.blocks.iter().find(|block| block.id == predecessor) else {
        return false;
    };
    let Some(instruction) = block
        .instructions
        .iter()
        .find(|instruction| instruction.id == comparison)
    else {
        return false;
    };
    let KirInstructionKind::Compare {
        op,
        left: left_value,
        right: right_value,
    } = instruction.kind
    else {
        return false;
    };
    let KirTerminator::Branch {
        condition,
        then_edge,
        else_edge,
    } = &block.terminator
    else {
        return false;
    };
    if instruction.results.first().map(|result| result.value) != Some(*condition)
        || left.value != left_value
        || right.value != right_value
        || if taken {
            then_edge.target != target
        } else {
            else_edge.target != target
        }
    {
        return false;
    }
    let Some(type_node) = value_integer_type(function, left.value) else {
        return false;
    };
    let (Ok(left_scalar), Ok(right_scalar)) = (
        ScalarValue::from_interval(type_node, left.interval.clone()),
        ScalarValue::from_interval(type_node, right.interval.clone()),
    ) else {
        return false;
    };
    let Ok((taken_values, other_values)) =
        refine_scalar_comparison(op, &left_scalar, &right_scalar)
    else {
        return false;
    };
    let expected = if taken { taken_values } else { other_values };
    (claim.value == left.value && claim.interval == *expected.0.interval())
        || (claim.value == right.value && claim.interval == *expected.1.interval())
}

fn loop_invariant_matches(
    function: &KirFunction,
    header: crate::BlockId,
    phi: ValueId,
    transfer: InstructionId,
    claim: &ScalarClaim,
) -> bool {
    let Some(header_block) = function.blocks.iter().find(|block| block.id == header) else {
        return false;
    };
    let Some(phi_index) = header_block
        .params
        .iter()
        .position(|param| param.value == phi)
    else {
        return false;
    };
    if claim.value != phi {
        return false;
    }
    let Some(type_node) = value_integer_type(function, phi) else {
        return false;
    };
    let Ok(invariant) = ScalarValue::from_interval(type_node, claim.interval.clone()) else {
        return false;
    };
    let dominators = compute_kir_dominators(function);
    let entry_values = predecessor_edges(function, header)
        .into_iter()
        .filter(|(predecessor, _)| !dominators.dominates(header, *predecessor))
        .filter_map(|(_, edge)| edge.args.get(phi_index).copied())
        .collect::<Vec<_>>();
    if entry_values.is_empty()
        || entry_values.iter().any(|value| {
            resolve_constant(function, *value).is_none_or(|entry| {
                entry < *claim.interval.lower() || entry > *claim.interval.upper()
            })
        })
    {
        return false;
    }
    let Some(instruction) = function
        .blocks
        .iter()
        .flat_map(|block| &block.instructions)
        .find(|instruction| instruction.id == transfer)
    else {
        return false;
    };
    let KirInstructionKind::Binary {
        op,
        left,
        right,
        semantics,
    } = instruction.kind
    else {
        return false;
    };
    let (other, phi_on_left) = if left == phi {
        (right, true)
    } else if right == phi {
        (left, false)
    } else {
        return false;
    };
    let Some(constant) = resolve_constant(function, other) else {
        return false;
    };
    let Ok(constant) = ScalarValue::constant(type_node, constant) else {
        return false;
    };
    let transfer = if phi_on_left {
        scalar_binary(op, semantics, &invariant, &constant)
    } else {
        scalar_binary(op, semantics, &constant, &invariant)
    };
    transfer.is_ok_and(|next| {
        next.failure() == ScalarFailure::None
            && next.interval().lower() >= claim.interval.lower()
            && next.interval().upper() <= claim.interval.upper()
    })
}

fn predecessor_edges(
    function: &KirFunction,
    target: crate::BlockId,
) -> Vec<(crate::BlockId, &crate::KirEdge)> {
    function
        .blocks
        .iter()
        .flat_map(|block| {
            match &block.terminator {
                KirTerminator::Return { .. } => Vec::new(),
                KirTerminator::Jump { edge } => vec![edge],
                KirTerminator::Branch {
                    then_edge,
                    else_edge,
                    ..
                } => vec![then_edge, else_edge],
            }
            .into_iter()
            .filter(move |edge| edge.target == target)
            .map(move |edge| (block.id, edge))
        })
        .collect()
}

fn resolve_constant(function: &KirFunction, value: ValueId) -> Option<BigInt> {
    function
        .blocks
        .iter()
        .flat_map(|block| &block.instructions)
        .find_map(|instruction| {
            (instruction.results.first().map(|result| result.value) == Some(value)).then_some(
                match &instruction.kind {
                    KirInstructionKind::ConstInt { value } => value.parse::<BigInt>().ok(),
                    _ => None,
                },
            )
        })
        .flatten()
}

fn value_integer_type(function: &KirFunction, value: ValueId) -> Option<super::IntegerType> {
    function
        .params
        .iter()
        .map(|param| (param.value, &param.type_node))
        .chain(function.blocks.iter().flat_map(|block| {
            block
                .params
                .iter()
                .map(|param| (param.value, &param.type_node))
                .chain(block.instructions.iter().flat_map(|instruction| {
                    instruction
                        .results
                        .iter()
                        .map(|result| (result.value, &result.type_node))
                }))
        }))
        .find_map(|(candidate, type_node)| {
            (candidate == value)
                .then(|| super::IntegerType::from_mir(type_node))
                .flatten()
        })
}

fn find_instruction(
    module: &KirModule,
    id: InstructionId,
) -> Option<(&KirFunction, &crate::KirBlock, &KirInstruction)> {
    module.functions.iter().find_map(|function| {
        function.blocks.iter().find_map(|block| {
            block
                .instructions
                .iter()
                .find(|instruction| instruction.id == id)
                .map(|instruction| (function, block, instruction))
        })
    })
}

fn fact_dependencies(derivation: &FactDerivation) -> Vec<FactId> {
    match derivation {
        FactDerivation::TrustedContractLeaf | FactDerivation::Constant { .. } => Vec::new(),
        FactDerivation::BinaryTransfer { inputs, .. } => inputs.clone(),
        FactDerivation::BranchRefinement { input, .. } => vec![*input],
        FactDerivation::LoopInvariant {
            entry, transfer, ..
        } => vec![*entry, *transfer],
    }
}

fn fact_error(fact: Option<FactId>, message: impl Into<String>) -> EvidenceValidationError {
    EvidenceValidationError {
        message: message.into(),
        fact,
        proof: None,
        step: None,
    }
}

fn proof_error(
    proof: Option<ProofId>,
    step: Option<u32>,
    message: impl Into<String>,
) -> EvidenceValidationError {
    EvidenceValidationError {
        message: message.into(),
        fact: None,
        proof,
        step,
    }
}
