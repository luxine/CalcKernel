use num_bigint::BigInt;

use crate::{
    FactId, InstructionId, KirArithmeticSemantics, KirCheckConditionKind, KirFunction,
    KirInstruction, KirInstructionKind, KirModule, KirTerminator, MirBinaryOp, MirUnaryOp, ProofId,
    ValueId, compute_kir_dominators,
};

use super::{
    ContractFactAffineExpression, ContractFactAffineTerm, ContractFactPredicate, ContractFactSet,
    FactArena, FactDerivation, FactOrigin, FactPredicate, ProofArena, ProofCertificate, ProofStep,
    ScalarAnalysisBudget, ScalarAnalysisResult, ScalarClaim, ScalarFailure, ScalarInterval,
    ScalarValue, contract_fact_dominates_at, refine_scalar_comparison, scalar_binary,
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
    Scalar(ScalarClaim, ScalarProofScope),
    Fact(FactPredicate, ScalarProofScope),
    Boolean(ValueId, bool, ScalarProofScope),
    GuardSafety,
    InductionEquality,
}

#[derive(Debug, Clone)]
enum ScalarProofScope {
    Everywhere,
    Block(crate::BlockId),
    Blocks(Vec<crate::BlockId>),
    Edge {
        predecessor: crate::BlockId,
        target: crate::BlockId,
        taken: bool,
    },
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
        ProofStep::TypeBounds { claim } => {
            let Some(ty) = value_integer_type(function, claim.value) else {
                return Err(prefix("type-bound value is not a KIR integer"));
            };
            if claim.interval != *ScalarValue::unknown(ty).interval()
                || claim.failure != ScalarFailure::None
            {
                return Err(prefix("type-bound claim is not the complete integer range"));
            }
            Ok(CheckedStep::Scalar(
                claim.clone(),
                ScalarProofScope::Everywhere,
            ))
        }
        ProofStep::ContractRange {
            block,
            premises,
            claim,
        } => {
            let Some(ty) = value_integer_type(function, claim.value) else {
                return Err(prefix("contract-range value is not a KIR integer"));
            };
            let premises = premises
                .iter()
                .map(|step| {
                    checked
                        .get(step.index() as usize)
                        .ok_or_else(|| prefix("contract-range premise is missing"))
                })
                .collect::<Result<Vec<_>, _>>()?;
            if !premises
                .iter()
                .all(|step| step_scope_allows_block(step, function, *block))
                || claim.failure != ScalarFailure::None
                || contract_interval_for_value(&premises, claim.value, ty).as_ref()
                    != Some(&claim.interval)
            {
                return Err(prefix(
                    "contract range is not justified at its declared block",
                ));
            }
            Ok(CheckedStep::Scalar(
                claim.clone(),
                ScalarProofScope::Block(*block),
            ))
        }
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
                    fact_scalar_scope(function, &fact_value.scope),
                )),
                FactPredicate::Contract(_) => Ok(CheckedStep::Fact(
                    fact_value.predicate.clone(),
                    fact_scalar_scope(function, &fact_value.scope),
                )),
            }
        }
        ProofStep::Constant { instruction, claim } => {
            let Some((owner, block, instruction)) = find_instruction(module, *instruction) else {
                return Err(prefix("constant instruction is missing"));
            };
            if owner.id != function.id || !constant_matches_claim(instruction, claim) {
                return Err(prefix("constant claim does not match KIR instruction"));
            }
            Ok(CheckedStep::Scalar(
                claim.clone(),
                ScalarProofScope::Block(block.id),
            ))
        }
        ProofStep::BinaryTransfer {
            instruction,
            left,
            right,
            claim,
        } => {
            let Some((owner, block, instruction)) = find_instruction(module, *instruction) else {
                return Err(prefix("binary instruction is missing"));
            };
            let left = checked_scalar_at(checked, *left, function, block.id, &prefix)?;
            let right = checked_scalar_at(checked, *right, function, block.id, &prefix)?;
            if owner.id != function.id || !binary_transfer_matches(instruction, left, right, claim)
            {
                return Err(prefix("binary claim does not match local transfer"));
            }
            Ok(CheckedStep::Scalar(
                claim.clone(),
                ScalarProofScope::Block(block.id),
            ))
        }
        ProofStep::CopyTransfer {
            instruction,
            input,
            claim,
        } => {
            let Some((owner, block, instruction)) = find_instruction(module, *instruction) else {
                return Err(prefix("copy instruction is missing"));
            };
            let input = checked_scalar_at(checked, *input, function, block.id, &prefix)?;
            if owner.id != function.id
                || !copy_transfer_matches(function, instruction, input, claim)
            {
                return Err(prefix("copy claim does not match local transfer"));
            }
            Ok(CheckedStep::Scalar(
                claim.clone(),
                ScalarProofScope::Block(block.id),
            ))
        }
        ProofStep::PhiJoin {
            block,
            inputs,
            claim,
        } => {
            let edges = predecessor_edges(function, *block);
            if edges.len() != inputs.len()
                || edges
                    .iter()
                    .zip(inputs)
                    .any(|((predecessor, edge), input)| {
                        checked.get(input.index() as usize).is_none_or(|step| {
                            !step_scope_allows_edge(step, function, *predecessor, edge)
                        })
                    })
            {
                return Err(prefix("phi input scope does not cover every incoming edge"));
            }
            let inputs = inputs
                .iter()
                .map(|step| checked_scalar(checked, step.index(), &prefix))
                .collect::<Result<Vec<_>, _>>()?;
            if !phi_join_matches(function, *block, &inputs, claim) {
                return Err(prefix("phi claim does not cover every incoming edge"));
            }
            Ok(CheckedStep::Scalar(
                claim.clone(),
                ScalarProofScope::Block(*block),
            ))
        }
        ProofStep::IntegerComparison {
            instruction,
            left,
            right,
            value,
            result,
        } => {
            let Some((owner, block, instruction)) = find_instruction(module, *instruction) else {
                return Err(prefix("comparison instruction is missing"));
            };
            let left = checked_scalar_at(checked, *left, function, block.id, &prefix)?;
            let right = checked_scalar_at(checked, *right, function, block.id, &prefix)?;
            if owner.id != function.id
                || !integer_comparison_matches(function, instruction, left, right, *value, *result)
            {
                return Err(prefix(
                    "integer comparison claim does not match local transfer",
                ));
            }
            Ok(CheckedStep::Boolean(
                *value,
                *result,
                ScalarProofScope::Block(block.id),
            ))
        }
        ProofStep::BooleanTransfer {
            instruction,
            inputs,
            value,
            result,
        } => {
            let Some((owner, block, instruction)) = find_instruction(module, *instruction) else {
                return Err(prefix("boolean instruction is missing"));
            };
            let inputs = inputs
                .iter()
                .map(|input| {
                    if checked
                        .get(input.index() as usize)
                        .is_none_or(|step| !step_scope_allows_block(step, function, block.id))
                    {
                        return Err(prefix("boolean premise is outside its proven scope"));
                    }
                    checked_boolean(checked, *input, &prefix)
                })
                .collect::<Result<Vec<_>, _>>()?;
            if owner.id != function.id
                || !boolean_transfer_matches(instruction, &inputs, *value, *result)
            {
                return Err(prefix("boolean claim does not match local transfer"));
            }
            Ok(CheckedStep::Boolean(
                *value,
                *result,
                ScalarProofScope::Block(block.id),
            ))
        }
        ProofStep::BooleanPhiJoin {
            block,
            inputs,
            value,
            result,
        } => {
            let target = function
                .blocks
                .iter()
                .find(|candidate| candidate.id == *block)
                .ok_or_else(|| prefix("boolean phi block is missing"))?;
            let index = target
                .params
                .iter()
                .position(|param| {
                    param.value == *value
                        && param.type_node
                            == crate::MirType::Primitive(crate::MirPrimitiveTypeName::Bool)
                })
                .ok_or_else(|| prefix("boolean phi parameter is missing or not bool"))?;
            let edges = predecessor_edges(function, *block);
            if edges.is_empty() || edges.len() != inputs.len() {
                return Err(prefix("boolean phi does not cover every incoming edge"));
            }
            for ((predecessor, edge), input) in edges.iter().zip(inputs) {
                let (input_value, input_result) = checked_boolean(checked, *input, &prefix)?;
                if edge.args.get(index) != Some(&input_value)
                    || input_result != *result
                    || checked.get(input.index() as usize).is_none_or(|step| {
                        !step_scope_allows_edge(step, function, *predecessor, edge)
                    })
                {
                    return Err(prefix(
                        "boolean phi value or scope does not cover every incoming edge",
                    ));
                }
            }
            Ok(CheckedStep::Boolean(
                *value,
                *result,
                ScalarProofScope::Block(*block),
            ))
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
            let left = checked_scalar_at(checked, *left, function, *predecessor, &prefix)?;
            let right = checked_scalar_at(checked, *right, function, *predecessor, &prefix)?;
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
            Ok(CheckedStep::Scalar(
                claim.clone(),
                ScalarProofScope::Edge {
                    predecessor: *predecessor,
                    target: *target,
                    taken: *taken,
                },
            ))
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
            Ok(CheckedStep::Scalar(
                claim.clone(),
                ScalarProofScope::Block(*header),
            ))
        }
        ProofStep::InductionEquality {
            header,
            left,
            right,
            pairs,
            definitions,
        } => {
            if proof.use_site.block != *header
                || !induction_equality_matches(function, *header, *left, *right, pairs, definitions)
            {
                return Err(prefix(
                    "induction equality is not closed over every entry and transfer",
                ));
            }
            Ok(CheckedStep::InductionEquality)
        }
        ProofStep::GuardSafety {
            condition_instruction,
            premises,
            allow_loop_reasoning,
        } => {
            let premises = premises
                .iter()
                .map(|premise| {
                    checked
                        .get(premise.index() as usize)
                        .ok_or_else(|| prefix("guard-safety premise is missing"))
                })
                .collect::<Result<Vec<_>, _>>()?;
            let Some((owner, block, _)) = find_instruction(module, *condition_instruction) else {
                return Err(prefix("guard-safety instruction is missing"));
            };
            if owner.id != function.id
                || !premises
                    .iter()
                    .all(|step| step_scope_allows_block(step, function, block.id))
            {
                return Err(prefix("guard-safety premise is outside its proven scope"));
            }
            if !guard_safety_matches(
                function,
                *condition_instruction,
                &premises,
                *allow_loop_reasoning,
            ) {
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
        Some(CheckedStep::Scalar(claim, _)) => Ok(claim),
        Some(
            CheckedStep::Fact(..)
            | CheckedStep::Boolean(..)
            | CheckedStep::GuardSafety
            | CheckedStep::InductionEquality,
        ) => Err(prefix("step dependency is not a scalar claim")),
        None => Err(prefix("step dependency is missing")),
    }
}

fn checked_scalar_at<'a>(
    checked: &'a [CheckedStep],
    step: super::ProofStepId,
    function: &KirFunction,
    block: crate::BlockId,
    prefix: &impl Fn(&str) -> String,
) -> Result<&'a ScalarClaim, String> {
    if checked
        .get(step.index() as usize)
        .is_none_or(|step| !step_scope_allows_block(step, function, block))
    {
        return Err(prefix("scalar premise is outside its proven scope"));
    }
    checked_scalar(checked, step.index(), prefix)
}

fn fact_scalar_scope(function: &KirFunction, scope: &super::FactScope) -> ScalarProofScope {
    match scope {
        super::FactScope::Block { block, .. } => ScalarProofScope::Block(*block),
        super::FactScope::InlineClone { blocks, .. } => ScalarProofScope::Blocks(blocks.clone()),
        super::FactScope::FunctionEntry(_) | super::FactScope::CalleeInstance { .. } => {
            function.blocks.first().map_or_else(
                || ScalarProofScope::Blocks(Vec::new()),
                |block| ScalarProofScope::Block(block.id),
            )
        }
    }
}

fn step_scope_allows_block(
    step: &CheckedStep,
    function: &KirFunction,
    block: crate::BlockId,
) -> bool {
    let (CheckedStep::Scalar(_, scope)
    | CheckedStep::Fact(_, scope)
    | CheckedStep::Boolean(_, _, scope)) = step
    else {
        return false;
    };
    match scope {
        ScalarProofScope::Everywhere => true,
        ScalarProofScope::Block(scope) => compute_kir_dominators(function).dominates(*scope, block),
        ScalarProofScope::Blocks(blocks) => blocks.contains(&block),
        ScalarProofScope::Edge { .. } => false,
    }
}

fn step_scope_allows_edge(
    step: &CheckedStep,
    function: &KirFunction,
    predecessor: crate::BlockId,
    edge: &crate::KirEdge,
) -> bool {
    if let CheckedStep::Scalar(
        _,
        ScalarProofScope::Edge {
            predecessor: scope_predecessor,
            target,
            taken,
        },
    ) = step
    {
        let Some(block) = function.blocks.iter().find(|block| block.id == predecessor) else {
            return false;
        };
        let KirTerminator::Branch {
            then_edge,
            else_edge,
            ..
        } = &block.terminator
        else {
            return false;
        };
        return *scope_predecessor == predecessor
            && *target == edge.target
            && std::ptr::eq(edge, if *taken { then_edge } else { else_edge });
    }
    step_scope_allows_block(step, function, predecessor)
}

fn guard_safety_matches(
    function: &KirFunction,
    condition_instruction: InstructionId,
    premises: &[&CheckedStep],
    allow_loop_reasoning: bool,
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
            if allow_loop_reasoning && strict_bound_increment_is_safe(function, instruction) {
                return true;
            }
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
                .is_some_and(|value| scalar_excludes(function, premises, *value, 0)),
            KirCheckConditionKind::SignedDivisionOverflow => {
                signed_division_is_safe(function, premises, args)
            }
            KirCheckConditionKind::SliceOutOfBounds => {
                slice_index_is_safe(function, premises, args)
                    || (allow_loop_reasoning
                        && strict_bound_slice_index_is_safe(
                            function,
                            instruction.id,
                            premises,
                            args,
                        ))
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
        CheckedStep::Scalar(claim, _) if claim.value == value => Some(claim),
        _ => None,
    }) {
        return ScalarValue::from_interval(type_node, claim.interval.clone())
            .ok()
            .map(|scalar| scalar.with_failure(claim.failure));
    }
    if let Some(interval) = contract_interval_for_value(premises, value, type_node) {
        return ScalarValue::from_interval(type_node, interval).ok();
    }
    resolve_constant(function, value)
        .and_then(|constant| ScalarValue::constant(type_node, constant).ok())
}

fn contract_interval_for_value(
    premises: &[&CheckedStep],
    value: ValueId,
    type_node: super::IntegerType,
) -> Option<ScalarInterval> {
    super::contract_scalar_interval(
        premises.iter().filter_map(|step| match step {
            CheckedStep::Fact(FactPredicate::Contract(predicate), _) => Some(predicate),
            _ => None,
        }),
        value,
        type_node,
    )
}

fn exact_scalar(
    function: &KirFunction,
    premises: &[&CheckedStep],
    value: ValueId,
) -> Option<BigInt> {
    resolve_constant(function, value).or_else(|| {
        premises.iter().find_map(|premise| match premise {
            CheckedStep::Scalar(claim, _)
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
    let Some(type_node) = value_integer_type(function, *left) else {
        return false;
    };
    if !type_node.is_signed() {
        return false;
    }
    scalar_excludes(function, premises, *left, type_node.minimum_i128())
        || scalar_excludes(function, premises, *right, -1)
}

fn scalar_excludes(
    function: &KirFunction,
    premises: &[&CheckedStep],
    value: ValueId,
    excluded: i128,
) -> bool {
    value_integer_type(function, value)
        .and_then(|ty| scalar_for_value(function, premises, value, ty))
        .is_some_and(|value| {
            let excluded = BigInt::from(excluded);
            value.failure() == ScalarFailure::None
                && (value.interval().upper() < &excluded || value.interval().lower() > &excluded)
        })
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
        value_integer_type(function, *index)
            .and_then(|ty| scalar_for_value(function, premises, *index, ty)),
        resolve_slice_len(function, premises, *slice),
    ) {
        return index.failure() == ScalarFailure::None
            && index.interval().lower() >= &BigInt::from(0)
            && index.interval().upper() < &len;
    }
    premises.iter().any(|premise| {
        matches!(
            premise,
            CheckedStep::Fact(FactPredicate::Contract(predicate), _)
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

fn contract_proves_value_at_most_slice_len(
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
    (operator == "<="
        && affine_is_single_term(left, ContractFactAffineTerm::Value(value))
        && affine_is_single_term(right, ContractFactAffineTerm::SliceLength(slice)))
        || (operator == ">="
            && affine_is_single_term(left, ContractFactAffineTerm::SliceLength(slice))
            && affine_is_single_term(right, ContractFactAffineTerm::Value(value)))
}

fn strict_bound_slice_index_is_safe(
    function: &KirFunction,
    condition_instruction: InstructionId,
    premises: &[&CheckedStep],
    args: &[ValueId],
) -> bool {
    let [slice, index] = args else {
        return false;
    };
    if value_integer_type(function, *index) != Some(super::IntegerType::U32) {
        return false;
    }
    let slice = forwarding_origin(function, *slice).unwrap_or(*slice);
    strict_upper_bounds(function, condition_instruction, *index)
        .into_iter()
        .any(|bound| {
            let bound = forwarding_origin(function, bound).unwrap_or(bound);
            value_is_slice_len_of(function, bound, slice)
                || premises.iter().any(|premise| {
                    matches!(
                        premise,
                        CheckedStep::Fact(FactPredicate::Contract(predicate), _)
                            if contract_proves_value_at_most_slice_len(predicate, bound, slice)
                    )
                })
        })
}

fn value_is_slice_len_of(function: &KirFunction, value: ValueId, slice: ValueId) -> bool {
    let Some(instruction) = function
        .blocks
        .iter()
        .flat_map(|block| &block.instructions)
        .find(|instruction| {
            instruction
                .results
                .iter()
                .any(|result| result.value == value)
        })
    else {
        return false;
    };
    let KirInstructionKind::SliceLen {
        slice: measured_slice,
    } = instruction.kind
    else {
        return false;
    };
    forwarded_from(function, measured_slice, slice)
        || forwarded_from(function, slice, measured_slice)
        || forwarding_origin(function, measured_slice)
            .is_some_and(|origin| forwarding_origin(function, slice) == Some(origin))
}

fn strict_bound_increment_is_safe(function: &KirFunction, instruction: &KirInstruction) -> bool {
    let KirInstructionKind::Binary {
        op: crate::MirBinaryOp::Add,
        left,
        right,
        semantics: KirArithmeticSemantics::Checked,
    } = instruction.kind
    else {
        return false;
    };
    let left_origin = forwarding_origin(function, left).unwrap_or(left);
    let right_origin = forwarding_origin(function, right).unwrap_or(right);
    let value = if resolve_constant(function, right_origin) == Some(BigInt::from(1)) {
        left
    } else if resolve_constant(function, left_origin) == Some(BigInt::from(1)) {
        right
    } else {
        return false;
    };
    // This is a pointwise rule, not an inferred induction invariant: on a
    // dominating i < bound edge, i + 1 <= bound <= the integer type's maximum.
    !strict_upper_bounds(function, instruction.id, value).is_empty()
}

fn strict_upper_bounds(
    function: &KirFunction,
    instruction: InstructionId,
    value: ValueId,
) -> Vec<ValueId> {
    let Some(use_block) = function.blocks.iter().find(|block| {
        block
            .instructions
            .iter()
            .any(|candidate| candidate.id == instruction)
    }) else {
        return Vec::new();
    };
    function
        .blocks
        .iter()
        .filter_map(|block| {
            let KirTerminator::Branch { condition, .. } = block.terminator else {
                return None;
            };
            let comparison = block.instructions.iter().find(|instruction| {
                instruction.results.first().map(|result| result.value) == Some(condition)
            })?;
            let KirInstructionKind::Compare { op, left, right } = comparison.kind else {
                return None;
            };
            let (tested, bound) = match op {
                crate::MirCompareOp::Lt => (left, right),
                crate::MirCompareOp::Gt => (right, left),
                _ => return None,
            };
            let type_node = value_integer_type(function, value)?;
            (value_integer_type(function, tested) == Some(type_node)
                && value_integer_type(function, bound) == Some(type_node)
                && forwarded_from(function, value, tested)
                && taken_edge_dominates(function, block.id, use_block.id))
            .then_some(bound)
        })
        .collect()
}

fn taken_edge_dominates(
    function: &KirFunction,
    branch: crate::BlockId,
    use_block: crate::BlockId,
) -> bool {
    let Some(entry) = function.blocks.first() else {
        return false;
    };
    // Remove this particular edge, not its target block. The other branch may
    // have the same target or reach the use through a different predecessor.
    let mut pending = vec![entry.id];
    let mut visited = std::collections::BTreeSet::new();
    while let Some(id) = pending.pop() {
        if id == use_block {
            return false;
        }
        if !visited.insert(id) {
            continue;
        }
        let Some(block) = function.blocks.iter().find(|block| block.id == id) else {
            return false;
        };
        match &block.terminator {
            KirTerminator::Return { .. } => {}
            KirTerminator::Jump { edge } => pending.push(edge.target),
            KirTerminator::Branch {
                then_edge,
                else_edge,
                ..
            } => {
                if id != branch {
                    pending.push(then_edge.target);
                }
                pending.push(else_edge.target);
            }
        }
    }
    visited.contains(&branch)
}

fn forwarded_from(function: &KirFunction, value: ValueId, origin: ValueId) -> bool {
    forwarding_leaves(function, value, Some(origin))
        .is_some_and(|leaves| leaves == std::collections::BTreeSet::from([origin]))
}

fn forwarding_origin(function: &KirFunction, value: ValueId) -> Option<ValueId> {
    let leaves = forwarding_leaves(function, value, None)?;
    (leaves.len() == 1)
        .then(|| leaves.first().copied())
        .flatten()
}

fn forwarding_leaves(
    function: &KirFunction,
    value: ValueId,
    stop_at: Option<ValueId>,
) -> Option<std::collections::BTreeSet<ValueId>> {
    let mut pending = vec![value];
    let mut visited = std::collections::BTreeSet::new();
    let mut leaves = std::collections::BTreeSet::new();
    while let Some(value) = pending.pop() {
        if !visited.insert(value) {
            continue;
        }
        if Some(value) == stop_at {
            leaves.insert(value);
        } else if let Some((block, index)) = function.blocks.iter().find_map(|block| {
            block
                .params
                .iter()
                .position(|param| param.value == value)
                .map(|index| (block.id, index))
        }) {
            let incoming = predecessor_edges(function, block);
            if incoming.is_empty() {
                return None;
            }
            for (_, edge) in incoming {
                pending.push(*edge.args.get(index)?);
            }
        } else if let Some(operand) = function
            .blocks
            .iter()
            .flat_map(|block| &block.instructions)
            .find_map(|instruction| {
                if instruction.results.first().map(|result| result.value) != Some(value) {
                    return None;
                }
                match instruction.kind {
                    KirInstructionKind::Copy { value } => Some(value),
                    _ => None,
                }
            })
        {
            pending.push(operand);
        } else {
            leaves.insert(value);
        }
    }
    Some(leaves)
}

/// Independently checks a transient SSA-identity claim before a pure instruction
/// is relocated. Every phi input and Copy must reach the claimed source; this
/// does not trust a loop analysis result or source-language slot names.
pub(crate) fn verify_ssa_forwarding(
    function: &KirFunction,
    value: ValueId,
    source: ValueId,
) -> bool {
    let type_of = |value| {
        function
            .params
            .iter()
            .map(|param| (param.value, &param.type_node))
            .chain(function.blocks.iter().flat_map(|block| {
                block
                    .params
                    .iter()
                    .map(|param| (param.value, &param.type_node))
            }))
            .chain(function.blocks.iter().flat_map(|block| {
                block.instructions.iter().flat_map(|instruction| {
                    instruction
                        .results
                        .iter()
                        .map(|result| (result.value, &result.type_node))
                })
            }))
            .find_map(|(candidate, ty)| (candidate == value).then_some(ty))
    };
    type_of(value).is_some()
        && type_of(value) == type_of(source)
        && forwarding_leaves(function, value, Some(source))
            .is_some_and(|leaves| leaves.len() == 1 && leaves.contains(&source))
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

fn checked_boolean(
    checked: &[CheckedStep],
    id: super::ProofStepId,
    prefix: &impl Fn(&str) -> String,
) -> Result<(ValueId, bool), String> {
    match checked.get(id.index() as usize) {
        Some(CheckedStep::Boolean(value, result, _)) => Ok((*value, *result)),
        _ => Err(prefix("boolean premise is missing or not a boolean claim")),
    }
}

fn boolean_transfer_matches(
    instruction: &KirInstruction,
    inputs: &[(ValueId, bool)],
    value: ValueId,
    expected: bool,
) -> bool {
    let [result] = instruction.results.as_slice() else {
        return false;
    };
    if result.value != value
        || result.type_node != crate::MirType::Primitive(crate::MirPrimitiveTypeName::Bool)
        || instruction.effect.is_some()
        || instruction.memory.is_some()
    {
        return false;
    }
    match (&instruction.kind, inputs) {
        (KirInstructionKind::ConstBool { value }, []) => *value == expected,
        (KirInstructionKind::Copy { value }, [(input, result)]) => {
            value == input && *result == expected
        }
        (
            KirInstructionKind::Unary {
                op: MirUnaryOp::Not,
                operand,
                ..
            },
            [(input, result)],
        ) => operand == input && *result != expected,
        (
            KirInstructionKind::Compare { op, left, right },
            [(left_id, left_value), (right_id, right_value)],
        ) if left == left_id && right == right_id => match op {
            crate::MirCompareOp::Eq => (*left_value == *right_value) == expected,
            crate::MirCompareOp::Ne => (*left_value != *right_value) == expected,
            _ => false,
        },
        _ => false,
    }
}

fn copy_transfer_matches(
    function: &KirFunction,
    instruction: &KirInstruction,
    input: &ScalarClaim,
    claim: &ScalarClaim,
) -> bool {
    let KirInstructionKind::Copy { value } = instruction.kind else {
        return false;
    };
    let [result] = instruction.results.as_slice() else {
        return false;
    };
    let Some(ty) = super::IntegerType::from_mir(&result.type_node) else {
        return false;
    };
    value == input.value
        && value_integer_type(function, value) == Some(ty)
        && result.value == claim.value
        && claim.interval == input.interval
        && claim.failure == input.failure
}

fn phi_join_matches(
    function: &KirFunction,
    block: crate::BlockId,
    inputs: &[&ScalarClaim],
    claim: &ScalarClaim,
) -> bool {
    let Some(block) = function
        .blocks
        .iter()
        .find(|candidate| candidate.id == block)
    else {
        return false;
    };
    let Some(index) = block
        .params
        .iter()
        .position(|param| param.value == claim.value)
    else {
        return false;
    };
    let Some(ty) = super::IntegerType::from_mir(&block.params[index].type_node) else {
        return false;
    };
    let edges = predecessor_edges(function, block.id);
    if edges.is_empty() || edges.len() != inputs.len() || claim.failure != ScalarFailure::None {
        return false;
    }
    let mut lower = None;
    let mut upper = None;
    for ((_, edge), input) in edges.iter().zip(inputs) {
        if edge.args.get(index) != Some(&input.value)
            || value_integer_type(function, input.value) != Some(ty)
            || input.failure != ScalarFailure::None
        {
            return false;
        }
        lower = Some(lower.map_or_else(
            || input.interval.lower().clone(),
            |value: BigInt| value.min(input.interval.lower().clone()),
        ));
        upper = Some(upper.map_or_else(
            || input.interval.upper().clone(),
            |value: BigInt| value.max(input.interval.upper().clone()),
        ));
    }
    lower.as_ref() == Some(claim.interval.lower()) && upper.as_ref() == Some(claim.interval.upper())
}

fn integer_comparison_matches(
    function: &KirFunction,
    instruction: &KirInstruction,
    left: &ScalarClaim,
    right: &ScalarClaim,
    value: ValueId,
    expected: bool,
) -> bool {
    let KirInstructionKind::Compare {
        op,
        left: left_id,
        right: right_id,
    } = instruction.kind
    else {
        return false;
    };
    let [result] = instruction.results.as_slice() else {
        return false;
    };
    let Some(ty) = value_integer_type(function, left_id) else {
        return false;
    };
    if result.value != value
        || result.type_node != crate::MirType::Primitive(crate::MirPrimitiveTypeName::Bool)
        || left.value != left_id
        || right.value != right_id
        || value_integer_type(function, right_id) != Some(ty)
        || left.failure != ScalarFailure::None
        || right.failure != ScalarFailure::None
    {
        return false;
    }
    let (Ok(left), Ok(right)) = (
        ScalarValue::from_interval(ty, left.interval.clone()),
        ScalarValue::from_interval(ty, right.interval.clone()),
    ) else {
        return false;
    };
    let expected = if expected {
        super::BoolLattice::AlwaysTrue
    } else {
        super::BoolLattice::AlwaysFalse
    };
    super::scalar_compare(op, &left, &right).is_ok_and(|result| result == expected)
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
    if left.failure != ScalarFailure::None
        || right.failure != ScalarFailure::None
        || claim.failure != ScalarFailure::None
    {
        return false;
    }
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
    if claim.value != phi || claim.failure != ScalarFailure::None {
        return false;
    }
    let Some(type_node) = value_integer_type(function, phi) else {
        return false;
    };
    let Ok(invariant) = ScalarValue::from_interval(type_node, claim.interval.clone()) else {
        return false;
    };
    let dominators = compute_kir_dominators(function);
    let incoming = predecessor_edges(function, header);
    let entry_values = incoming
        .iter()
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
    let Some(result) = instruction.results.first() else {
        return false;
    };
    let backedges = incoming
        .iter()
        .filter(|(predecessor, _)| dominators.dominates(header, *predecessor))
        .collect::<Vec<_>>();
    if backedges.is_empty()
        || super::IntegerType::from_mir(&result.type_node) != Some(type_node)
        || backedges
            .iter()
            .any(|(_, edge)| edge.args.get(phi_index) != Some(&result.value))
    {
        return false;
    }
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

fn induction_equality_matches(
    function: &KirFunction,
    header: crate::BlockId,
    left: ValueId,
    right: ValueId,
    pairs: &[(ValueId, ValueId)],
    definitions: &[InstructionId],
) -> bool {
    let Some(block) = function.blocks.iter().find(|block| block.id == header) else {
        return false;
    };
    if left >= right
        || !block.params.iter().any(|param| param.value == left)
        || !block.params.iter().any(|param| param.value == right)
        || pairs.binary_search(&(left, right)).is_err()
        || pairs.windows(2).any(|pair| pair[0] >= pair[1])
        || pairs.len()
            > ScalarAnalysisBudget::for_function(function, super::ScalarAnalysisConfig::default())
                .max_steps() as usize
    {
        return false;
    }
    let dominators = compute_kir_dominators(function);
    let incoming = predecessor_edges(function, header);
    if !incoming
        .iter()
        .any(|(predecessor, _)| dominators.dominates(header, *predecessor))
        || !incoming
            .iter()
            .any(|(predecessor, _)| !dominators.dominates(header, *predecessor))
    {
        return false;
    }
    let equivalent =
        |a: ValueId, b: ValueId| a == b || pairs.binary_search(&(a.min(b), a.max(b))).is_ok();
    let mut expected_definitions = std::collections::BTreeSet::new();
    for &(left, right) in pairs {
        let Some(ty) = value_integer_type(function, left) else {
            return false;
        };
        if left >= right || value_integer_type(function, right) != Some(ty) {
            return false;
        }
        let instruction = |value| {
            function
                .blocks
                .iter()
                .flat_map(|block| &block.instructions)
                .find(|instruction| {
                    instruction
                        .results
                        .first()
                        .is_some_and(|result| result.value == value)
                })
        };
        let a = instruction(left);
        let b = instruction(right);
        expected_definitions.extend(a.iter().chain(b.iter()).map(|instruction| instruction.id));
        if let Some(KirInstruction {
            kind: KirInstructionKind::Copy { value },
            ..
        }) = a
        {
            if !equivalent(*value, right) {
                return false;
            }
            continue;
        }
        if let Some(KirInstruction {
            kind: KirInstructionKind::Copy { value },
            ..
        }) = b
        {
            if !equivalent(left, *value) {
                return false;
            }
            continue;
        }
        let parameter = |value| {
            function.blocks.iter().find_map(|block| {
                block
                    .params
                    .iter()
                    .position(|param| param.value == value)
                    .map(|index| (block.id, index))
            })
        };
        if let (Some((a_block, a_index)), Some((b_block, b_index))) =
            (parameter(left), parameter(right))
        {
            if a_block != b_block {
                return false;
            }
            let edges = predecessor_edges(function, a_block);
            if edges.is_empty()
                || edges.iter().any(|(_, edge)| {
                    match (edge.args.get(a_index), edge.args.get(b_index)) {
                        (Some(a), Some(b)) => !equivalent(*a, *b),
                        _ => true,
                    }
                })
            {
                return false;
            }
            continue;
        }
        let (Some(a), Some(b)) = (a, b) else {
            return false;
        };
        if a.memory.is_some() || b.memory.is_some() || a.effect.is_some() || b.effect.is_some() {
            return false;
        }
        match (&a.kind, &b.kind) {
            (
                KirInstructionKind::ConstInt { value: a },
                KirInstructionKind::ConstInt { value: b },
            ) => {
                let (Ok(a), Ok(b)) = (a.parse::<BigInt>(), b.parse::<BigInt>()) else {
                    return false;
                };
                if a != b || ScalarValue::constant(ty, a).is_err() {
                    return false;
                }
            }
            (
                KirInstructionKind::Binary {
                    op: a_op,
                    left: a_left,
                    right: a_right,
                    semantics: a_semantics,
                },
                KirInstructionKind::Binary {
                    op: b_op,
                    left: b_left,
                    right: b_right,
                    semantics: b_semantics,
                },
            ) => {
                if !matches!(a_op, MirBinaryOp::Add | MirBinaryOp::Sub)
                    || a_op != b_op
                    || a_semantics != b_semantics
                    || *a_semantics == KirArithmeticSemantics::StrictFloat
                    || !equivalent(*a_left, *b_left)
                    || !equivalent(*a_right, *b_right)
                {
                    return false;
                }
            }
            _ => return false,
        }
    }
    expected_definitions.into_iter().collect::<Vec<_>>() == definitions
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
