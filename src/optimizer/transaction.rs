use sha2::{Digest, Sha256};

use crate::{ContractFactSet, KirModule, MemoryVersionId, ValueId, print_kir_module};

use super::{
    CandidateBudgetCharge, CandidateDisposition, CandidateKey, KirGuardElimination,
    KirOptimizationAuditState, ProofArena, kir_function_units, print_proof_arena,
    validate_kir_optimization_evidence,
};

/// Every append-only identity cursor that belongs to a rollback-capable KIR state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KirIdAllocators {
    pub next_function: u32,
    pub next_block: u32,
    pub next_value: u32,
    pub next_instruction: u32,
    pub next_memory_region: u32,
    pub next_memory_version: u32,
    pub next_vector_region: u32,
    pub next_loop: u32,
    pub next_proof: u32,
}

impl KirIdAllocators {
    fn for_state(module: &KirModule, proofs: &ProofArena) -> Result<Self, String> {
        fn next(maximum: Option<u32>, kind: &str) -> Result<u32, String> {
            maximum
                .map_or(Some(0), |value| value.checked_add(1))
                .ok_or_else(|| format!("KIR {kind} identity space is exhausted"))
        }

        Ok(Self {
            next_function: next(
                module
                    .functions
                    .iter()
                    .map(|function| function.id.index())
                    .max(),
                "function",
            )?,
            next_block: next(
                module
                    .functions
                    .iter()
                    .flat_map(|function| &function.blocks)
                    .map(|block| block.id.index())
                    .max(),
                "block",
            )?,
            next_value: next(
                module
                    .functions
                    .iter()
                    .flat_map(|function| {
                        function.params.iter().map(|param| param.value).chain(
                            function.blocks.iter().flat_map(|block| {
                                block.params.iter().map(|param| param.value).chain(
                                    block.instructions.iter().flat_map(|instruction| {
                                        instruction.results.iter().map(|result| result.value)
                                    }),
                                )
                            }),
                        )
                    })
                    .map(ValueId::index)
                    .max(),
                "value",
            )?,
            next_instruction: next(
                module
                    .functions
                    .iter()
                    .flat_map(|function| &function.blocks)
                    .flat_map(|block| &block.instructions)
                    .map(|instruction| instruction.id.index())
                    .max(),
                "instruction",
            )?,
            next_memory_region: next(
                module
                    .functions
                    .iter()
                    .flat_map(|function| &function.regions)
                    .map(|region| region.id.index())
                    .max(),
                "memory-region",
            )?,
            next_memory_version: next(
                module
                    .functions
                    .iter()
                    .flat_map(|function| {
                        function
                            .initial_memory
                            .iter()
                            .map(|memory| memory.version)
                            .chain(function.blocks.iter().flat_map(|block| {
                                block.memory_params.iter().map(|param| param.version).chain(
                                    block.instructions.iter().flat_map(|instruction| {
                                        instruction
                                            .memory
                                            .iter()
                                            .flat_map(|memory| [Some(memory.input), memory.output])
                                            .flatten()
                                    }),
                                )
                            }))
                    })
                    .map(MemoryVersionId::index)
                    .max(),
                "memory-version",
            )?,
            next_vector_region: next(
                module
                    .functions
                    .iter()
                    .flat_map(|function| &function.vector_regions)
                    .map(|region| region.id.index())
                    .max(),
                "vector-region",
            )?,
            next_loop: next(
                proofs
                    .proofs()
                    .iter()
                    .flat_map(|proof| &proof.steps)
                    .filter_map(|step| match step {
                        super::ProofStep::CanonicalLoop { loop_id, .. } => Some(loop_id.index()),
                        _ => None,
                    })
                    .max(),
                "loop",
            )?,
            next_proof: next(
                proofs.proofs().iter().map(|proof| proof.id.index()).max(),
                "proof",
            )?,
        })
    }
}

/// Identity of the exact state accepted by the structural/evidence verifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KirVerificationCacheIdentity {
    pub kir_digest: String,
    pub evidence_digest: String,
}

/// The complete rollback domain for one optimizer transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KirVerifiedProgramState {
    module: KirModule,
    contract_facts: Option<ContractFactSet>,
    proofs: ProofArena,
    eliminated_guards: Vec<KirGuardElimination>,
    evidence_generation: u32,
    optimization_entry_module_units: u32,
    ids: KirIdAllocators,
    verification_cache: KirVerificationCacheIdentity,
}

impl KirVerifiedProgramState {
    pub fn new(
        module: KirModule,
        contract_facts: Option<ContractFactSet>,
        evidence_generation: u32,
    ) -> Result<Self, String> {
        Self::from_parts(
            module,
            contract_facts,
            ProofArena::new(evidence_generation),
            Vec::new(),
            evidence_generation,
        )
    }

    pub fn from_parts(
        module: KirModule,
        contract_facts: Option<ContractFactSet>,
        proofs: ProofArena,
        eliminated_guards: Vec<KirGuardElimination>,
        evidence_generation: u32,
    ) -> Result<Self, String> {
        let validation = validate_kir_optimization_evidence(
            &module,
            contract_facts.as_ref(),
            &proofs,
            &eliminated_guards,
            evidence_generation,
        );
        if !validation.errors.is_empty() {
            return Err(validation
                .errors
                .into_iter()
                .map(|error| error.message)
                .collect::<Vec<_>>()
                .join("; "));
        }
        let optimization_entry_module_units =
            module.functions.iter().fold(0_u32, |total, function| {
                total.saturating_add(kir_function_units(function))
            });
        Self::from_verified_parts(
            module,
            contract_facts,
            proofs,
            eliminated_guards,
            evidence_generation,
            optimization_entry_module_units,
        )
    }

    pub(crate) fn from_verified_parts(
        module: KirModule,
        contract_facts: Option<ContractFactSet>,
        proofs: ProofArena,
        eliminated_guards: Vec<KirGuardElimination>,
        evidence_generation: u32,
        optimization_entry_module_units: u32,
    ) -> Result<Self, String> {
        let ids = KirIdAllocators::for_state(&module, &proofs)?;
        let verification_cache = cache_identity(
            &module,
            contract_facts.as_ref(),
            &proofs,
            &eliminated_guards,
            evidence_generation,
        );
        Ok(Self {
            module,
            contract_facts,
            proofs,
            eliminated_guards,
            evidence_generation,
            optimization_entry_module_units,
            ids,
            verification_cache,
        })
    }

    #[must_use]
    pub const fn module(&self) -> &KirModule {
        &self.module
    }

    pub fn module_mut(&mut self) -> &mut KirModule {
        self.verification_cache.kir_digest.clear();
        &mut self.module
    }

    #[must_use]
    pub const fn contract_facts(&self) -> Option<&ContractFactSet> {
        self.contract_facts.as_ref()
    }

    pub fn contract_facts_mut(&mut self) -> Option<&mut ContractFactSet> {
        self.verification_cache.evidence_digest.clear();
        self.contract_facts.as_mut()
    }

    #[must_use]
    pub const fn proofs(&self) -> &ProofArena {
        &self.proofs
    }

    pub fn proofs_mut(&mut self) -> &mut ProofArena {
        self.verification_cache.evidence_digest.clear();
        &mut self.proofs
    }

    #[must_use]
    pub fn eliminated_guards(&self) -> &[KirGuardElimination] {
        &self.eliminated_guards
    }

    pub fn eliminated_guards_mut(&mut self) -> &mut Vec<KirGuardElimination> {
        self.verification_cache.evidence_digest.clear();
        &mut self.eliminated_guards
    }

    #[must_use]
    pub const fn evidence_generation(&self) -> u32 {
        self.evidence_generation
    }

    #[must_use]
    pub const fn optimization_entry_module_units(&self) -> u32 {
        self.optimization_entry_module_units
    }

    #[must_use]
    pub const fn ids(&self) -> KirIdAllocators {
        self.ids
    }

    pub(crate) fn fresh_function(&mut self) -> Result<crate::FunctionId, String> {
        allocate_id(
            &mut self.ids.next_function,
            crate::FunctionId::from_index,
            "function",
        )
    }

    pub(crate) fn fresh_block(&mut self) -> Result<crate::BlockId, String> {
        allocate_id(
            &mut self.ids.next_block,
            crate::BlockId::from_index,
            "block",
        )
    }

    pub(crate) fn fresh_value(&mut self) -> Result<crate::ValueId, String> {
        allocate_id(
            &mut self.ids.next_value,
            crate::ValueId::from_index,
            "value",
        )
    }

    pub(crate) fn fresh_instruction(&mut self) -> Result<crate::InstructionId, String> {
        allocate_id(
            &mut self.ids.next_instruction,
            crate::InstructionId::from_index,
            "instruction",
        )
    }

    pub(crate) fn fresh_memory_region(&mut self) -> Result<crate::MemoryRegionId, String> {
        allocate_id(
            &mut self.ids.next_memory_region,
            crate::MemoryRegionId::from_index,
            "memory-region",
        )
    }

    pub(crate) fn fresh_memory_version(&mut self) -> Result<crate::MemoryVersionId, String> {
        allocate_id(
            &mut self.ids.next_memory_version,
            crate::MemoryVersionId::from_index,
            "memory-version",
        )
    }

    pub(crate) fn fresh_vector_region(&mut self) -> Result<crate::VectorRegionId, String> {
        allocate_id(
            &mut self.ids.next_vector_region,
            crate::VectorRegionId::from_index,
            "vector-region",
        )
    }

    #[must_use]
    pub const fn verification_cache(&self) -> &KirVerificationCacheIdentity {
        &self.verification_cache
    }

    #[must_use]
    pub fn kir_digest(&self) -> String {
        if self.verification_cache.kir_digest.is_empty() {
            digest(print_kir_module(&self.module).as_bytes())
        } else {
            self.verification_cache.kir_digest.clone()
        }
    }

    fn revalidated(self) -> Result<Self, String> {
        let validation = validate_kir_optimization_evidence(
            &self.module,
            self.contract_facts.as_ref(),
            &self.proofs,
            &self.eliminated_guards,
            self.evidence_generation,
        );
        if !validation.errors.is_empty() {
            return Err(validation
                .errors
                .into_iter()
                .map(|error| error.message)
                .collect::<Vec<_>>()
                .join("; "));
        }
        Self::from_verified_parts(
            self.module,
            self.contract_facts,
            self.proofs,
            self.eliminated_guards,
            self.evidence_generation,
            self.optimization_entry_module_units,
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransactionCheckError {
    Reject(String),
    Compiler(String),
}

impl TransactionCheckError {
    #[must_use]
    pub fn reject(message: impl Into<String>) -> Self {
        Self::Reject(message.into())
    }

    #[must_use]
    pub fn compiler(message: impl Into<String>) -> Self {
        Self::Compiler(message.into())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransactionOutcome {
    Committed,
    Rejected,
    BudgetExhausted,
    CompilerError(String),
}

pub fn execute_verified_transaction<Mutate, Check>(
    state: &mut KirVerifiedProgramState,
    audit: &mut KirOptimizationAuditState,
    key: CandidateKey,
    charge: CandidateBudgetCharge,
    mutate: Mutate,
    check: Check,
) -> TransactionOutcome
where
    Mutate: FnOnce(&mut KirVerifiedProgramState) -> Result<(), String>,
    Check: FnOnce(
        &KirVerifiedProgramState,
        &KirVerifiedProgramState,
    ) -> Result<(), TransactionCheckError>,
{
    execute_verified_transaction_with_disposition(
        state,
        audit,
        key,
        charge,
        CandidateDisposition::Accepted,
        mutate,
        check,
    )
}

pub(crate) fn execute_verified_transaction_with_disposition<Mutate, Check>(
    state: &mut KirVerifiedProgramState,
    audit: &mut KirOptimizationAuditState,
    key: CandidateKey,
    charge: CandidateBudgetCharge,
    success_disposition: CandidateDisposition,
    mutate: Mutate,
    check: Check,
) -> TransactionOutcome
where
    Mutate: FnOnce(&mut KirVerifiedProgramState) -> Result<(), String>,
    Check: FnOnce(
        &KirVerifiedProgramState,
        &KirVerifiedProgramState,
    ) -> Result<(), TransactionCheckError>,
{
    if !matches!(
        success_disposition,
        CandidateDisposition::Accepted | CandidateDisposition::Reused
    ) {
        return TransactionOutcome::CompilerError(
            "transaction success disposition is invalid".to_string(),
        );
    }
    if audit.contains_key(&key) {
        return TransactionOutcome::CompilerError(format!(
            "duplicate optimizer candidate key: {}",
            key.stable_text()
        ));
    }
    if !audit.ledger_mut().try_debit(&charge) {
        let reason = "budget-exhausted";
        if let Err(error) =
            audit.append_attempt(key, charge, CandidateDisposition::BudgetExhausted, reason)
        {
            return TransactionOutcome::CompilerError(error);
        }
        return TransactionOutcome::BudgetExhausted;
    }

    let pre_state = state.clone();
    let mut trial = pre_state.clone();
    if let Err(error) = mutate(&mut trial) {
        let _ = audit.append_attempt(
            key,
            charge,
            CandidateDisposition::CompilerError,
            error.clone(),
        );
        return TransactionOutcome::CompilerError(error);
    }
    match check(&pre_state, &trial) {
        Err(TransactionCheckError::Reject(reason)) => {
            if let Err(error) =
                audit.append_attempt(key, charge, CandidateDisposition::Rejected, reason)
            {
                return TransactionOutcome::CompilerError(error);
            }
            TransactionOutcome::Rejected
        }
        Err(TransactionCheckError::Compiler(error)) => {
            let _ = audit.append_attempt(
                key,
                charge,
                CandidateDisposition::CompilerError,
                error.clone(),
            );
            TransactionOutcome::CompilerError(error)
        }
        Ok(()) => match trial.revalidated() {
            Err(error) => {
                let _ = audit.append_attempt(
                    key,
                    charge,
                    CandidateDisposition::CompilerError,
                    error.clone(),
                );
                TransactionOutcome::CompilerError(error)
            }
            Ok(verified_trial) => {
                if let Err(error) = audit.append_attempt(
                    key,
                    charge,
                    success_disposition,
                    success_disposition.stable_name(),
                ) {
                    return TransactionOutcome::CompilerError(error);
                }
                *state = verified_trial;
                TransactionOutcome::Committed
            }
        },
    }
}

fn cache_identity(
    module: &KirModule,
    contracts: Option<&ContractFactSet>,
    proofs: &ProofArena,
    eliminated_guards: &[KirGuardElimination],
    generation: u32,
) -> KirVerificationCacheIdentity {
    let kir_text = print_kir_module(module);
    let evidence = format!(
        "generation={generation}\ncontracts={contracts:?}\n{}guards={eliminated_guards:?}",
        print_proof_arena(proofs)
    );
    KirVerificationCacheIdentity {
        kir_digest: digest(kir_text.as_bytes()),
        evidence_digest: digest(evidence.as_bytes()),
    }
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn allocate_id<Id>(
    cursor: &mut u32,
    constructor: impl FnOnce(u32) -> Id,
    kind: &str,
) -> Result<Id, String> {
    let current = *cursor;
    *cursor = cursor
        .checked_add(1)
        .ok_or_else(|| format!("KIR {kind} identity space is exhausted"))?;
    Ok(constructor(current))
}
