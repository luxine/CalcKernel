use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fmt,
};

use crate::{BlockId, FactId, InstructionId, ProofId, ValueId};

use super::{FactUseSite, ScalarFailure, ScalarInterval};

/// Stable local identity of a step inside one proof certificate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProofStepId(u32);

impl ProofStepId {
    #[must_use]
    pub const fn from_index(index: u32) -> Self {
        Self(index)
    }

    #[must_use]
    pub const fn index(self) -> u32 {
        self.0
    }
}

/// Scalar conclusion produced by a locally checked certificate step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScalarClaim {
    pub value: ValueId,
    pub interval: ScalarInterval,
    pub failure: ScalarFailure,
}

impl ScalarClaim {
    #[must_use]
    pub const fn new(value: ValueId, interval: ScalarInterval, failure: ScalarFailure) -> Self {
        Self {
            value,
            interval,
            failure,
        }
    }
}

/// Closed proof language accepted by the independent checker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProofStep {
    TypeBounds {
        claim: ScalarClaim,
    },
    ContractRange {
        block: BlockId,
        premises: Vec<ProofStepId>,
        claim: ScalarClaim,
    },
    FactLeaf {
        fact: FactId,
    },
    Constant {
        instruction: InstructionId,
        claim: ScalarClaim,
    },
    BinaryTransfer {
        instruction: InstructionId,
        left: ProofStepId,
        right: ProofStepId,
        claim: ScalarClaim,
    },
    CopyTransfer {
        instruction: InstructionId,
        input: ProofStepId,
        claim: ScalarClaim,
    },
    PhiJoin {
        block: BlockId,
        inputs: Vec<ProofStepId>,
        claim: ScalarClaim,
    },
    IntegerComparison {
        instruction: InstructionId,
        left: ProofStepId,
        right: ProofStepId,
        value: ValueId,
        result: bool,
    },
    BranchRefinement {
        predecessor: BlockId,
        target: BlockId,
        comparison: InstructionId,
        left: ProofStepId,
        right: ProofStepId,
        taken: bool,
        claim: ScalarClaim,
    },
    LoopInvariant {
        header: BlockId,
        phi: ValueId,
        transfer: InstructionId,
        claim: ScalarClaim,
    },
    GuardSafety {
        condition_instruction: InstructionId,
        premises: Vec<ProofStepId>,
        allow_loop_reasoning: bool,
    },
}

impl ProofStep {
    fn dependencies(&self) -> Vec<ProofStepId> {
        match self {
            Self::TypeBounds { .. }
            | Self::FactLeaf { .. }
            | Self::Constant { .. }
            | Self::LoopInvariant { .. } => Vec::new(),
            Self::CopyTransfer { input, .. } => vec![*input],
            Self::PhiJoin { inputs, .. } => inputs.clone(),
            Self::BinaryTransfer { left, right, .. }
            | Self::IntegerComparison { left, right, .. }
            | Self::BranchRefinement { left, right, .. } => vec![*left, *right],
            Self::GuardSafety { premises, .. } | Self::ContractRange { premises, .. } => {
                premises.clone()
            }
        }
    }
}

/// One immutable closed proof used at a specific KIR site.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProofCertificate {
    pub id: ProofId,
    pub generation: u32,
    pub use_site: FactUseSite,
    pub steps: Vec<ProofStep>,
    pub root: ProofStepId,
}

impl ProofCertificate {
    /// Keep only the closed dependency DAG of these roots, in original order.
    pub(crate) fn project_steps(
        &self,
        roots: &[ProofStepId],
    ) -> Result<(Vec<ProofStep>, Vec<ProofStepId>), ProofArenaError> {
        let mut needed = BTreeSet::new();
        let mut work = roots.to_vec();
        while let Some(id) = work.pop() {
            if !needed.insert(id) {
                continue;
            }
            let step = self
                .steps
                .get(id.index() as usize)
                .ok_or_else(|| ProofArenaError::new("projected proof step is missing"))?;
            for dependency in step.dependencies() {
                if dependency >= id {
                    return Err(ProofArenaError::new(
                        "projected proof dependency is not earlier",
                    ));
                }
                work.push(dependency);
            }
        }
        let mapping = needed
            .iter()
            .enumerate()
            .map(|(index, id)| {
                u32::try_from(index)
                    .map(|index| (*id, ProofStepId::from_index(index)))
                    .map_err(|_| ProofArenaError::new("projected proof exceeds u32 identity space"))
            })
            .collect::<Result<BTreeMap<_, _>, _>>()?;
        let mut steps = Vec::with_capacity(needed.len());
        for id in needed {
            let mut step = self.steps[id.index() as usize].clone();
            match &mut step {
                ProofStep::CopyTransfer { input, .. } => *input = mapping[input],
                ProofStep::BinaryTransfer { left, right, .. }
                | ProofStep::IntegerComparison { left, right, .. }
                | ProofStep::BranchRefinement { left, right, .. } => {
                    *left = mapping[left];
                    *right = mapping[right];
                }
                ProofStep::PhiJoin {
                    inputs: premises, ..
                }
                | ProofStep::ContractRange { premises, .. }
                | ProofStep::GuardSafety { premises, .. } => {
                    for premise in premises {
                        *premise = mapping[premise];
                    }
                }
                ProofStep::TypeBounds { .. }
                | ProofStep::FactLeaf { .. }
                | ProofStep::Constant { .. }
                | ProofStep::LoopInvariant { .. } => {}
            }
            steps.push(step);
        }
        Ok((steps, roots.iter().map(|root| mapping[root]).collect()))
    }
}

/// Append-only proof storage with stable identity order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProofArena {
    generation: u32,
    proofs: Vec<ProofCertificate>,
}

impl ProofArena {
    #[must_use]
    pub const fn new(generation: u32) -> Self {
        Self {
            generation,
            proofs: Vec::new(),
        }
    }

    #[must_use]
    pub const fn generation(&self) -> u32 {
        self.generation
    }

    #[must_use]
    pub fn proofs(&self) -> &[ProofCertificate] {
        &self.proofs
    }

    #[must_use]
    pub fn get(&self, id: ProofId) -> Option<&ProofCertificate> {
        self.proofs.get(id.index() as usize)
    }

    pub fn get_mut(&mut self, id: ProofId) -> Option<&mut ProofCertificate> {
        self.proofs.get_mut(id.index() as usize)
    }

    pub(crate) fn instruction_dependencies(&self) -> BTreeSet<InstructionId> {
        self.proofs
            .iter()
            .flat_map(|proof| &proof.steps)
            .filter_map(|step| match step {
                ProofStep::Constant { instruction, .. }
                | ProofStep::BinaryTransfer { instruction, .. }
                | ProofStep::CopyTransfer { instruction, .. }
                | ProofStep::IntegerComparison { instruction, .. } => Some(*instruction),
                ProofStep::BranchRefinement { comparison, .. } => Some(*comparison),
                ProofStep::LoopInvariant { transfer, .. } => Some(*transfer),
                ProofStep::GuardSafety {
                    condition_instruction,
                    ..
                } => Some(*condition_instruction),
                ProofStep::TypeBounds { .. }
                | ProofStep::FactLeaf { .. }
                | ProofStep::ContractRange { .. }
                | ProofStep::PhiJoin { .. } => None,
            })
            .collect()
    }

    pub(crate) fn block_parameter_dependencies(&self) -> BTreeSet<ValueId> {
        self.proofs
            .iter()
            .flat_map(|proof| &proof.steps)
            .filter_map(|step| match step {
                ProofStep::PhiJoin { claim, .. } => Some(claim.value),
                ProofStep::LoopInvariant { phi, .. } => Some(*phi),
                _ => None,
            })
            .collect()
    }

    pub fn try_insert(
        &mut self,
        use_site: FactUseSite,
        steps: Vec<ProofStep>,
        root: ProofStepId,
    ) -> Result<ProofId, ProofArenaError> {
        if steps.get(root.index() as usize).is_none() {
            return Err(ProofArenaError::new(format!(
                "proof root step{} is not defined",
                root.index()
            )));
        }
        for (index, step) in steps.iter().enumerate() {
            for dependency in step.dependencies() {
                if dependency.index() as usize >= index {
                    return Err(ProofArenaError::new(format!(
                        "proof step{index} dependency step{} is not earlier in the certificate",
                        dependency.index()
                    )));
                }
            }
        }
        let index = u32::try_from(self.proofs.len())
            .map_err(|_| ProofArenaError::new("proof arena exceeds u32 identity space"))?;
        let id = ProofId::from_index(index);
        self.proofs.push(ProofCertificate {
            id,
            generation: self.generation,
            use_site,
            steps,
            root,
        });
        Ok(id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProofArenaError {
    message: String,
}

impl ProofArenaError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for ProofArenaError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.message.fmt(formatter)
    }
}

impl Error for ProofArenaError {}

#[must_use]
pub fn print_proof_arena(arena: &ProofArena) -> String {
    let mut output = format!("proofs generation={}\n", arena.generation);
    for proof in &arena.proofs {
        output.push_str(&format!(
            "proof{} use=f{},b{} root=step{}\n",
            proof.id.index(),
            proof.use_site.function.index(),
            proof.use_site.block.index(),
            proof.root.index()
        ));
        for (index, step) in proof.steps.iter().enumerate() {
            output.push_str(&format!("  step{index} {}\n", print_step(step)));
        }
    }
    output
}

fn print_step(step: &ProofStep) -> String {
    match step {
        ProofStep::TypeBounds { claim } => format!("type-bounds {}", print_claim(claim)),
        ProofStep::ContractRange {
            block,
            premises,
            claim,
        } => format!(
            "contract-range b{} [{}] {}",
            block.index(),
            premises
                .iter()
                .map(|step| format!("step{}", step.index()))
                .collect::<Vec<_>>()
                .join(", "),
            print_claim(claim)
        ),
        ProofStep::FactLeaf { fact } => format!("fact fact{}", fact.index()),
        ProofStep::Constant { instruction, claim } => {
            format!("constant i{} {}", instruction.index(), print_claim(claim))
        }
        ProofStep::BinaryTransfer {
            instruction,
            left,
            right,
            claim,
        } => format!(
            "binary i{} step{} step{} {}",
            instruction.index(),
            left.index(),
            right.index(),
            print_claim(claim)
        ),
        ProofStep::CopyTransfer {
            instruction,
            input,
            claim,
        } => format!(
            "copy i{} step{} {}",
            instruction.index(),
            input.index(),
            print_claim(claim)
        ),
        ProofStep::PhiJoin {
            block,
            inputs,
            claim,
        } => format!(
            "phi b{} [{}] {}",
            block.index(),
            inputs
                .iter()
                .map(|step| format!("step{}", step.index()))
                .collect::<Vec<_>>()
                .join(", "),
            print_claim(claim)
        ),
        ProofStep::IntegerComparison {
            instruction,
            left,
            right,
            value,
            result,
        } => format!(
            "integer-compare i{} step{} step{} v{}={result}",
            instruction.index(),
            left.index(),
            right.index(),
            value.index()
        ),
        ProofStep::BranchRefinement {
            predecessor,
            target,
            comparison,
            left,
            right,
            taken,
            claim,
        } => format!(
            "branch b{}->b{} i{} step{} step{} taken={} {}",
            predecessor.index(),
            target.index(),
            comparison.index(),
            left.index(),
            right.index(),
            taken,
            print_claim(claim)
        ),
        ProofStep::LoopInvariant {
            header,
            phi,
            transfer,
            claim,
        } => format!(
            "invariant b{} v{} i{} {}",
            header.index(),
            phi.index(),
            transfer.index(),
            print_claim(claim)
        ),
        ProofStep::GuardSafety {
            condition_instruction,
            premises,
            allow_loop_reasoning,
        } => format!(
            "guard-safety i{} loop={} [{}]",
            condition_instruction.index(),
            allow_loop_reasoning,
            premises
                .iter()
                .map(|premise| format!("step{}", premise.index()))
                .collect::<Vec<_>>()
                .join(",")
        ),
    }
}

fn print_claim(claim: &ScalarClaim) -> String {
    format!(
        "claim(v{},{}..={},failure={:?})",
        claim.value.index(),
        claim.interval.lower(),
        claim.interval.upper(),
        claim.failure
    )
}
