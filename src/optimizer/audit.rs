use std::collections::{BTreeMap, BTreeSet};

use crate::{BlockId, FunctionId, InstructionId, KirFunction, KirModule, LoopId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LoopCandidateKind {
    LoopSimd,
    FullUnroll,
    PartialUnroll,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LoopCandidateVariant {
    Scalar,
    Slp,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum CandidateKey {
    Specialization {
        caller: FunctionId,
        call: InstructionId,
        callee: FunctionId,
        fact_set_digest: String,
    },
    LoopFrontier {
        function: FunctionId,
        loop_id: LoopId,
        kind: LoopCandidateKind,
        variant: LoopCandidateVariant,
        vf: u16,
        uf: u8,
    },
    ResidualSlp {
        function: FunctionId,
        block: BlockId,
        root: InstructionId,
        lanes: u16,
    },
}

impl CandidateKey {
    #[must_use]
    pub fn stable_text(&self) -> String {
        match self {
            Self::Specialization {
                caller,
                call,
                callee,
                fact_set_digest,
            } => format!(
                "specialization:f{}:i{}:f{}:{fact_set_digest}",
                caller.index(),
                call.index(),
                callee.index()
            ),
            Self::LoopFrontier {
                function,
                loop_id,
                kind,
                variant,
                vf,
                uf,
            } => format!(
                "loop:f{}:loop{}:{}:{}:vf{vf}:uf{uf}",
                function.index(),
                loop_id.index(),
                loop_kind_name(*kind),
                loop_variant_name(*variant)
            ),
            Self::ResidualSlp {
                function,
                block,
                root,
                lanes,
            } => format!(
                "residual-slp:f{}:b{}:i{}:lanes{lanes}",
                function.index(),
                block.index(),
                root.index()
            ),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandidateDisposition {
    Accepted,
    Rejected,
    Reused,
    NonWinner,
    BudgetExhausted,
    CompilerError,
}

impl CandidateDisposition {
    #[must_use]
    pub const fn stable_name(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Rejected => "rejected",
            Self::Reused => "reused",
            Self::NonWinner => "non-winner",
            Self::BudgetExhausted => "budget-exhausted",
            Self::CompilerError => "compiler-error",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateBudgetCharge {
    pub functions: Vec<FunctionId>,
    pub proposer_units: u32,
    pub checker_units: u32,
}

impl CandidateBudgetCharge {
    #[must_use]
    pub fn single(function: FunctionId, proposer_units: u32, checker_units: u32) -> Self {
        Self {
            functions: vec![function],
            proposer_units,
            checker_units,
        }
    }

    fn functions(&self) -> Option<BTreeSet<FunctionId>> {
        let functions = self.functions.iter().copied().collect::<BTreeSet<_>>();
        (!functions.is_empty() && functions.len() == self.functions.len()).then_some(functions)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FunctionOptimizationBudget {
    pub proposer_initial: u32,
    pub proposer_remaining: u32,
    pub checker_initial: u32,
    pub checker_remaining: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OptimizationBudgetLedger {
    functions: BTreeMap<FunctionId, FunctionOptimizationBudget>,
    owners: BTreeMap<FunctionId, FunctionId>,
}

impl OptimizationBudgetLedger {
    #[must_use]
    pub fn for_module(module: &KirModule) -> Self {
        let owners = module
            .functions
            .iter()
            .map(|function| (function.id, function.id))
            .collect();
        Self {
            functions: module
                .functions
                .iter()
                .map(|function| {
                    let units = kir_function_units(function);
                    let proposer = units.saturating_mul(64).saturating_add(128);
                    let checker = units.saturating_mul(96).saturating_add(256);
                    (
                        function.id,
                        FunctionOptimizationBudget {
                            proposer_initial: proposer,
                            proposer_remaining: proposer,
                            checker_initial: checker,
                            checker_remaining: checker,
                        },
                    )
                })
                .collect(),
            owners,
        }
    }

    #[must_use]
    pub fn budget(&self, function: FunctionId) -> Option<FunctionOptimizationBudget> {
        self.owners
            .get(&function)
            .and_then(|owner| self.functions.get(owner))
            .copied()
    }

    pub fn register_clone(
        &mut self,
        clone: FunctionId,
        original: FunctionId,
    ) -> Result<(), String> {
        let owner = self
            .owners
            .get(&original)
            .copied()
            .ok_or_else(|| "optimizer clone budget original is missing".to_string())?;
        if self.owners.insert(clone, owner).is_some() {
            return Err("optimizer clone budget identity is already registered".to_string());
        }
        Ok(())
    }

    pub fn try_debit(&mut self, charge: &CandidateBudgetCharge) -> bool {
        let Some(charged_functions) = charge.functions() else {
            return false;
        };
        let Some(functions) = charged_functions
            .iter()
            .map(|function| self.owners.get(function).copied())
            .collect::<Option<BTreeSet<_>>>()
        else {
            return false;
        };
        if functions.iter().any(|owner| {
            self.functions.get(owner).is_none_or(|budget| {
                budget.proposer_remaining < charge.proposer_units
                    || budget.checker_remaining < charge.checker_units
            })
        }) {
            return false;
        }
        for function in functions {
            let budget = self
                .functions
                .get_mut(&function)
                .expect("preflight checked every budget owner");
            budget.proposer_remaining -= charge.proposer_units;
            budget.checker_remaining -= charge.checker_units;
        }
        true
    }
}

#[must_use]
pub fn kir_function_units(function: &KirFunction) -> u32 {
    let units = function.params.len()
        + function.regions.len()
        + function.initial_memory.len()
        + function.vector_regions.len()
        + function.blocks.iter().fold(0, |count, block| {
            count + 1 + block.params.len() + block.memory_params.len() + block.instructions.len()
        });
    u32::try_from(units).unwrap_or(u32::MAX)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OptimizationAttempt {
    pub sequence: u32,
    pub key: CandidateKey,
    pub disposition: CandidateDisposition,
    pub reason: String,
    pub charge: CandidateBudgetCharge,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KirOptimizationAuditState {
    ledger: OptimizationBudgetLedger,
    attempts: Vec<OptimizationAttempt>,
    keys: BTreeSet<CandidateKey>,
    accepted: u32,
    rejected: u32,
}

impl KirOptimizationAuditState {
    #[must_use]
    pub fn for_module(module: &KirModule) -> Self {
        Self {
            ledger: OptimizationBudgetLedger::for_module(module),
            attempts: Vec::new(),
            keys: BTreeSet::new(),
            accepted: 0,
            rejected: 0,
        }
    }

    #[must_use]
    pub const fn ledger(&self) -> &OptimizationBudgetLedger {
        &self.ledger
    }

    pub fn ledger_mut(&mut self) -> &mut OptimizationBudgetLedger {
        &mut self.ledger
    }

    #[must_use]
    pub fn attempts(&self) -> &[OptimizationAttempt] {
        &self.attempts
    }

    #[must_use]
    pub const fn accepted(&self) -> u32 {
        self.accepted
    }

    #[must_use]
    pub const fn rejected(&self) -> u32 {
        self.rejected
    }

    pub fn register_clone_budget(
        &mut self,
        clone: FunctionId,
        original: FunctionId,
    ) -> Result<(), String> {
        self.ledger.register_clone(clone, original)
    }

    #[must_use]
    pub fn contains_key(&self, key: &CandidateKey) -> bool {
        self.keys.contains(key)
    }

    pub fn record_noncommitting_attempt(
        &mut self,
        key: CandidateKey,
        charge: CandidateBudgetCharge,
        disposition: CandidateDisposition,
        reason: impl Into<String>,
    ) -> Result<(), String> {
        if !matches!(
            disposition,
            CandidateDisposition::Rejected
                | CandidateDisposition::Reused
                | CandidateDisposition::NonWinner
        ) {
            return Err("noncommitting audit disposition is invalid".to_string());
        }
        let (disposition, reason) = if self.ledger.try_debit(&charge) {
            (disposition, reason.into())
        } else {
            (
                CandidateDisposition::BudgetExhausted,
                "budget-exhausted".to_string(),
            )
        };
        self.append_attempt(key, charge, disposition, reason)
    }

    pub(crate) fn append_attempt(
        &mut self,
        key: CandidateKey,
        charge: CandidateBudgetCharge,
        disposition: CandidateDisposition,
        reason: impl Into<String>,
    ) -> Result<(), String> {
        if !self.keys.insert(key.clone()) {
            return Err(format!(
                "duplicate optimizer candidate key: {}",
                key.stable_text()
            ));
        }
        let sequence = u32::try_from(self.attempts.len())
            .map_err(|_| "optimizer audit sequence exceeds u32 identity space")?;
        match disposition {
            CandidateDisposition::Accepted => self.accepted = self.accepted.saturating_add(1),
            CandidateDisposition::Rejected
            | CandidateDisposition::BudgetExhausted
            | CandidateDisposition::CompilerError => {
                self.rejected = self.rejected.saturating_add(1);
            }
            CandidateDisposition::Reused | CandidateDisposition::NonWinner => {}
        }
        self.attempts.push(OptimizationAttempt {
            sequence,
            key,
            disposition,
            reason: reason.into(),
            charge,
        });
        Ok(())
    }
}

pub fn order_candidate_keys(
    keys: impl IntoIterator<Item = CandidateKey>,
) -> Result<Vec<CandidateKey>, String> {
    let mut ordered = BTreeSet::new();
    for key in keys {
        if !ordered.insert(key.clone()) {
            return Err(format!(
                "duplicate optimizer candidate key: {}",
                key.stable_text()
            ));
        }
    }
    Ok(ordered.into_iter().collect())
}

#[must_use]
pub fn print_optimization_audit(audit: &KirOptimizationAuditState) -> String {
    let mut output = format!(
        "optimizer-audit accepted={} rejected={} attempts={}\n",
        audit.accepted,
        audit.rejected,
        audit.attempts.len()
    );
    for attempt in &audit.attempts {
        output.push_str(&format!(
            "attempt{} {} {} proposer={} checker={} reason={}\n",
            attempt.sequence,
            attempt.key.stable_text(),
            attempt.disposition.stable_name(),
            attempt.charge.proposer_units,
            attempt.charge.checker_units,
            attempt.reason
        ));
    }
    output
}

const fn loop_kind_name(kind: LoopCandidateKind) -> &'static str {
    match kind {
        LoopCandidateKind::LoopSimd => "loop-simd",
        LoopCandidateKind::FullUnroll => "full-unroll",
        LoopCandidateKind::PartialUnroll => "partial-unroll",
    }
}

const fn loop_variant_name(variant: LoopCandidateVariant) -> &'static str {
    match variant {
        LoopCandidateVariant::Scalar => "scalar",
        LoopCandidateVariant::Slp => "slp",
    }
}
