use std::{error::Error, fmt};

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
            Self::FactLeaf { .. } | Self::Constant { .. } | Self::LoopInvariant { .. } => {
                Vec::new()
            }
            Self::BinaryTransfer { left, right, .. }
            | Self::BranchRefinement { left, right, .. } => vec![*left, *right],
            Self::GuardSafety { premises, .. } => premises.clone(),
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
